import ast
import asyncio
import builtins
import dis
import inspect
import textwrap
from collections.abc import Callable
from typing import Any, Protocol, runtime_checkable

from serverlessworkflow.sdk import Workflow
from serverlessworkflow.sdk.base import Input, Output, Schema, TaskItem
from serverlessworkflow.sdk.tasks import (
    ContainerConfiguration,
    DoTask,
    ForConfiguration,
    ForTask,
    ForkConfiguration,
    ForkTask,
    RunConfiguration,
    RunTask,
    ScriptConfiguration,
    ShellConfiguration,
)
from serverlessworkflow.sdk.workflow import Document

from ._jackdaw import DurableEngineBuilder, ExecutionHandle

# Appended to every extracted function.
# Reads the full context from stdin as JSON, extracts only the named parameters,
# calls the function with kwargs, and prints the return value as JSON.
_MAIN_BLOCK = """
if __name__ == "__main__":
    import sys as _sys
    import json as _json
    _ctx = _json.load(_sys.stdin)
    _result = {fn_name}(**{{k: _ctx[k] for k in {params!r} if k in _ctx}})
    if _result is not None:
        print(_json.dumps(_result))
"""

# Merges the task's input context with its output dict so each step accumulates
# all values produced so far. $input is the task's input; . is the raw output.
_MERGE_OUTPUT = "$input + ."

# Shell/container steps with no JSON output just pass the context through unchanged.
_PASSTHROUGH_OUTPUT = "$input"

_BUILTIN_NAMES = frozenset(dir(builtins))

# Maps Python primitive type annotations to JSON Schema type strings.
_PY_TO_JSON_SCHEMA: dict[type, dict[str, str]] = {
    str: {"type": "string"},
    int: {"type": "integer"},
    float: {"type": "number"},
    bool: {"type": "boolean"},
    dict: {"type": "object"},
    list: {"type": "array"},
}


def _input_schema(fn: Callable) -> Input:
    """Build an Input with JSON Schema derived from fn's type annotations.

    Uses input.from_ to scope the task's input to exactly the keys the function
    declares, so the schema and the actual data passed to stdin are in sync.
    The expression selects each named parameter from the accumulated context.
    """
    sig = inspect.signature(fn)
    params = list(sig.parameters.keys())

    properties: dict[str, Any] = {}
    for name, param in sig.parameters.items():
        ann = param.annotation
        properties[name] = _PY_TO_JSON_SCHEMA.get(ann, {}) if ann is not inspect.Parameter.empty else {}

    schema_doc = {
        "type": "object",
        "properties": properties,
        "required": params,
    }

    # input.from_ selects only the declared parameters from the full context.
    # e.g. for fn(working_dir, url) -> "{working_dir: .working_dir, url: .url}"
    selections = ", ".join(f"{p}: .{p}" for p in params)
    from_expr = "{" + selections + "}"

    return Input(
        schema=Schema(document=schema_doc),
        from_=from_expr,
    )


def _needed_global_names(fn: Callable) -> set[str]:
    """Return global names that fn loads via LOAD_GLOBAL and are not builtins."""
    loaded = {
        instr.argval
        for instr in dis.get_instructions(fn)
        if instr.opname == "LOAD_GLOBAL"
    }
    fn_globals = fn.__globals__
    return {name for name in loaded if name in fn_globals and name not in _BUILTIN_NAMES}


def _imports_from_source(module_source: str, needed: set[str]) -> list[str]:
    """Extract import statements from module source text (AST-based).

    Narrows `from x import a, b` to only the names actually needed.
    Skips relative imports since they cannot be resolved outside their package.
    """
    module_ast = ast.parse(module_source)
    imports: list[str] = []
    seen: set[str] = set()

    for node in ast.iter_child_nodes(module_ast):
        if isinstance(node, ast.Import):
            for alias in node.names:
                bound = alias.asname or alias.name.split(".")[0]
                if bound in needed and bound not in seen:
                    imports.append(ast.unparse(node))
                    seen.add(bound)
                    break

        elif isinstance(node, ast.ImportFrom):
            if node.level > 0:
                continue
            relevant = [
                a for a in node.names
                if (a.asname or a.name) in needed and (a.asname or a.name) not in seen
            ]
            if relevant:
                narrowed = ast.ImportFrom(module=node.module, names=relevant, level=0)
                imports.append(ast.unparse(narrowed))
                for alias in relevant:
                    seen.add(alias.asname or alias.name)

    return imports


def _imports_from_globals(fn_globals: dict[str, Any], needed: set[str]) -> list[str]:
    """Reconstruct import statements from live objects using dill.

    Used as a fallback when module source is unavailable (notebooks, REPL, -c strings).
    dill.source.getimport resolves the correct public module path for each object,
    handling aliases, `from x import y`, and stdlib re-exports like os.path correctly.
    """
    import dill.source

    imports: list[str] = []
    for name in sorted(needed):
        val = fn_globals.get(name)
        if val is None:
            continue
        try:
            stmt = dill.source.getimport(val, alias=name)
            if stmt:
                imports.append(stmt.strip())
        except Exception:
            pass
    return imports


def _collect_helpers(
    fn: Callable,
    fn_module: Any,
    fn_globals: dict[str, Any],
    visited: set[str],
    result: list[str],
) -> None:
    """Post-order DFS: collect source for module-local helper functions fn calls.

    Processes dependencies before the functions that depend on them so the
    generated script defines helpers in the right order.
    """
    for name in _needed_global_names(fn):
        if name in visited:
            continue
        visited.add(name)
        val = fn_globals.get(name)
        if val is None or not callable(val):
            continue
        if inspect.getmodule(val) is not fn_module:
            continue
        try:
            _collect_helpers(val, fn_module, fn_globals, visited, result)
            result.append(textwrap.dedent(inspect.getsource(val)))
        except (OSError, TypeError):
            pass


def _extract_helper_functions(fn: Callable) -> list[str]:
    """Return source snippets for module-local helper functions that fn depends on.

    Handles transitive dependencies via post-order DFS so each helper is defined
    before its callers. Skips builtins and names that come from imports.
    """
    fn_module = inspect.getmodule(fn)
    if fn_module is None:
        return []
    result: list[str] = []
    visited: set[str] = {fn.__name__}
    _collect_helpers(fn, fn_module, fn.__globals__, visited, result)
    return result


def _extract_module_imports(fn: Callable) -> list[str]:
    """Return import statements for globals that fn references.

    Strategy 1 — AST source parsing: reads the module's .py file, finds the exact
    import statements, narrows multi-name imports to only what's needed.

    Strategy 2 — dill runtime reconstruction: used when source is unavailable (Jupyter
    notebooks, interactive REPL, python -c).
    """
    needed = _needed_global_names(fn)
    if not needed:
        return []

    module = inspect.getmodule(fn)
    if module is not None:
        try:
            return _imports_from_source(inspect.getsource(module), needed)
        except (OSError, TypeError):
            pass

    return _imports_from_globals(fn.__globals__, needed)


def container_task(
    image: str,
    command: str | None = None,
    arguments: list[str] | None = None,
    environment: dict[str, str] | None = None,
    volumes: dict[str, Any] | None = None,
    ports: dict[int, int] | None = None,
    name: str | None = None,
    output_as: str | None = None,
) -> RunTask:
    """Standalone factory for a reusable container step.

    Volume keys and values may be JQ expressions evaluated against the context at runtime:

        valhalla_container = container_task(
            image="ghcr.io/valhalla/valhalla:run-latest",
            command="valhalla_build_tiles -c /data/valhalla.json /data/scoped.osm.pbf",
            volumes={"${ .working_dir }": "/data"},
        )

    Use `name` to assign a stable container name — required for the fork-as-service
    pattern where a sibling branch stops the container by name.
    Use `ports` to publish ports: {container_port: host_port}.
    """
    return RunTask(
        run=RunConfiguration(
            container=ContainerConfiguration(
                image=image,
                name=name,
                command=command,
                arguments=arguments,
                environment=environment,
                volumes=volumes,
                ports=ports,
            ),
        ),
        output=Output(as_=output_as or _PASSTHROUGH_OUTPUT),
    )


def build_image_task(
    tag: str,
    dockerfile: str,
    context_dir: str = "",
) -> RunTask:
    """Generate a Python script step that builds a Docker image via jackdaw's bollard runtime.

    tag          — image tag to apply, e.g. "sedona-spark4:local"
    dockerfile   — Dockerfile content as a string
    context_dir  — path to the build context directory; pass "" to use a temp dir
                   containing only the Dockerfile

    Respects .dockerignore in context_dir. Build output streams to stderr in real time.
    The step passes the workflow context through unchanged.
    """
    code = (
        "import sys\n"
        "import jackdaw._jackdaw as _jd\n"
        "\n"
        "if __name__ == '__main__':\n"
        f"    print('Building image {tag}...', flush=True)\n"
        f"    _jd.build_image({tag!r}, {dockerfile!r}, {context_dir!r})\n"
        f"    print('Image {tag} built successfully.', flush=True)\n"
    )

    return RunTask(
        run=RunConfiguration(
            script=ScriptConfiguration(
                language="python",
                code=code,
                stdin="${ . }",
            ),
        ),
        output=Output(as_=_PASSTHROUGH_OUTPUT),
    )


def stop_container_task(container_name: str) -> RunTask:
    """Generate a Python script step that stops a named container via jackdaw's bollard runtime.

    container_name — literal container name, e.g. "sedona".

    The generated step calls `jackdaw._jackdaw.stop_container(name)` in a subprocess using the
    same interpreter (and virtualenv) that launched the pipeline — no docker CLI required.
    """
    code = textwrap.dedent(f"""\
        import sys
        import json
        import jackdaw._jackdaw as _jd

        if __name__ == "__main__":
            _jd.stop_container({container_name!r})
    """)

    return RunTask(
        run=RunConfiguration(
            script=ScriptConfiguration(
                language="python",
                code=code,
                stdin="${ . }",
            ),
        ),
        output=Output(as_=_PASSTHROUGH_OUTPUT),
    )


def shell_task(
    command: str,
    arguments: list[str] | None = None,
    environment: dict[str, str] | None = None,
    output_as: str | None = None,
) -> RunTask:
    """Standalone factory for a reusable shell step.

    Use this to define named, reusable command specs as plain Python functions.
    Pass JQ expressions as arguments at call-site to bind to different context keys:

        def osmium_extract(polygon: str, input_pbf: str, output_pbf: str) -> RunTask:
            return shell_task("osmium", ["extract", "-p", polygon, input_pbf, "-o", output_pbf])

        builder.add("extract-region", osmium_extract(
            polygon="${ .unified_division_geojson }",
            input_pbf="${ .raw_pbf_file }",
            output_pbf="${ .working_dir + \\"/valhalla_runs/scoped.osm.pbf\\" }",
        ))
    """
    return RunTask(
        run=RunConfiguration(
            shell=ShellConfiguration(
                command=command,
                arguments=arguments,
                environment=environment,
            ),
        ),
        output=Output(as_=output_as or _PASSTHROUGH_OUTPUT),
    )


class _EngineLoop:
    """Persistent background event loop + WorkflowRunner, shared across all .run() calls.

    asyncio.run() tears down the event loop after each call. When a cached
    DurableEngine is re-used on a fresh loop its internal Tokio futures deadlock.
    This class keeps one loop running forever in a daemon thread so every engine
    call lands on the same loop and the in-memory cache stays warm.
    """

    def __init__(self) -> None:
        import asyncio
        import atexit
        import threading

        self._loop = asyncio.new_event_loop()
        self._runner = WorkflowRunner()
        ready = threading.Event()

        def _run() -> None:
            asyncio.set_event_loop(self._loop)
            ready.set()
            self._loop.run_forever()

        t = threading.Thread(target=_run, daemon=True)
        t.start()
        ready.wait()

        # On interpreter exit: stop the asyncio loop, then force-exit after 3 s
        # so Tokio runtime threads can't block the process and stall kernel restart.
        def _shutdown() -> None:
            import os, threading
            self._loop.call_soon_threadsafe(self._loop.stop)
            t = threading.Timer(3.0, os._exit, args=(0,))
            t.daemon = True
            t.start()

        atexit.register(_shutdown)

    def run(self, workflow: Workflow, ctx: dict, timeout: float) -> dict:
        import asyncio

        future = asyncio.run_coroutine_threadsafe(
            self._runner.run_async(workflow, input=ctx, timeout=timeout),
            self._loop,
        )
        result = future.result(timeout + 30)
        return dict(result)


class WorkflowBuilder:
    """Fluent builder that assembles a Workflow from Python functions, shell commands, and containers."""

    def __init__(self, name: str = "", namespace: str = "default", version: str = "0.1.0") -> None:
        self._name = name
        self._namespace = namespace
        self._version = version
        self._steps: list[TaskItem] = []
        # Lazily-created persistent engine loop; all .run() calls share it so
        # the in-memory cache is warm across cells and event loop reuse is safe.
        self._engine_loop: "_EngineLoop | None" = None

    def add(self, name: str, fn_or_task: "Callable | RunTask") -> "WorkflowBuilder":
        """Add a step by name.

        Accepts either a plain Python callable or a pre-built ``RunTask``
        (from ``shell_task``, ``container_task``, ``mkdir``, etc.):

            pipeline.add("filter-feeds", filter_feeds)          # callable
            pipeline.add("make-dir",     mkdir("${ .working_dir }"))  # RunTask

        For Python callables, module-level imports are detected and prepended
        automatically.  The function receives its declared parameters by name
        from the accumulated context (not positional argv), so non-string
        types are preserved.  Its return dict is merged into the context for
        subsequent steps.
        """
        if callable(fn_or_task):
            fn = fn_or_task
            params = list(inspect.signature(fn).parameters.keys())
            imports = _extract_module_imports(fn)
            helpers = _extract_helper_functions(fn)
            source = textwrap.dedent(inspect.getsource(fn))
            header = "\n".join(imports) + "\n\n" if imports else ""
            helper_block = "\n\n".join(helpers) + "\n\n" if helpers else ""
            code = header + helper_block + source + _MAIN_BLOCK.format(fn_name=fn.__name__, params=params)
            task: RunTask = RunTask(
                run=RunConfiguration(
                    script=ScriptConfiguration(
                        language="python",
                        code=code,
                        stdin="${ . }",
                    ),
                ),
                input=_input_schema(fn),
                output=Output(as_=_MERGE_OUTPUT),
            )
            self._steps.append(TaskItem(name=name, task=task))
        else:
            self._steps.append(TaskItem(name=name, task=fn_or_task))
        return self

    def run_shell(
        self,
        name: str,
        command: str,
        arguments: list[str] | None = None,
        environment: dict[str, str] | None = None,
        output_as: str | None = None,
    ) -> "WorkflowBuilder":
        """Add a shell command step.

        command  — the executable name (e.g. "mkdir", "aria2c", "osmium").
        arguments — list of arguments; each entry is either a literal string or a
                    standalone JQ expression evaluated against the context, e.g.:
                        "${ .working_dir }"
                        "${ .working_dir + \\"/valhalla_runs\\" }"

        By default the context passes through unchanged (the shell step is a side
        effect). Pass output_as with a JQ expression to merge computed values into
        the context — useful when the output path is deterministic from the inputs:

            .run_shell("extract", "osmium", arguments=[...],
                       output_as="${ $input + {scoped_pbf: ($input.working_dir + '/out.pbf')} }")
        """
        task = RunTask(
            run=RunConfiguration(
                shell=ShellConfiguration(
                    command=command,
                    arguments=arguments,
                    environment=environment,
                ),
            ),
            output=Output(as_=output_as or _PASSTHROUGH_OUTPUT),
        )
        self._steps.append(TaskItem(name=name, task=task))
        return self

    def run_container(
        self,
        name: str,
        image: str,
        command: str | None = None,
        arguments: list[str] | None = None,
        environment: dict[str, str] | None = None,
        volumes: dict[str, Any] | None = None,
        ports: dict[int, int] | None = None,
        container_name: str | None = None,
        output_as: str | None = None,
    ) -> "WorkflowBuilder":
        """Add a container step.

        command        — shell script run inside the container via sh -c (supports redirects).
        arguments      — positional $1/$2/… args passed after the command.
        volumes        — host→container path mappings; keys/values may be JQ expressions,
                         e.g. volumes={"${ .working_dir }": "/data"}.
        ports          — port mappings: {container_port: host_port}.
        container_name — stable Docker container name; required when a sibling fork branch
                         needs to stop this container by name.
        output_as      — JQ expression for the output transform; defaults to passthrough.
        """
        task = container_task(
            image=image,
            command=command,
            arguments=arguments,
            environment=environment,
            volumes=volumes,
            ports=ports,
            name=container_name,
            output_as=output_as,
        )
        self._steps.append(TaskItem(name=name, task=task))
        return self

    def for_loop(
        self,
        name: str,
        each: str,
        in_: str,
        body: "WorkflowBuilder",
    ) -> "WorkflowBuilder":
        """Add a step that iterates over a collection, executing body steps per item.

        each — variable name injected into context for each item (e.g. "scenario")
        in_  — raw JQ expression evaluating to the array (e.g. ".scenarios")
               Do NOT use the ${ } wrapper here — in_ is a runtime expression
               field that is passed directly to the JQ evaluator.
        body — WorkflowBuilder whose steps run on each iteration

        Each iteration's output is merged into context so subsequent steps and
        later iterations accumulate results. The loop variable is removed from
        context after each iteration.
        """
        self._steps.append(TaskItem(
            name=name,
            task=ForTask(
                for_=ForConfiguration(each=each, in_=in_),
                do=body.steps(),
            ),
        ))
        return self

    def fork(self, name: str, branches: dict[str, "WorkflowBuilder"]) -> "WorkflowBuilder":
        """Add parallel branches. Each value is a WorkflowBuilder defining that branch."""
        branch_items = [
            TaskItem(name=branch_name, task=DoTask(do=sub.steps()))
            for branch_name, sub in branches.items()
        ]
        self._steps.append(TaskItem(name=name, task=ForkTask(fork=ForkConfiguration(branches=branch_items))))
        return self

    def run(self, ctx: dict, timeout: float = 3600.0) -> dict:
        """Execute all accumulated steps against the given context.

        In a Jupyter/IPython session: runs the full assembled workflow through
        the jackdaw engine, returns the resulting context dict. The engine
        instance is reused across calls so the in-memory cache persists —
        re-running the same cell with unchanged inputs is instant.

        Outside an interactive session: returns ``ctx`` unchanged so that
        module-level ``.run()`` calls are no-ops when the file is executed as
        a script. ``__main__`` handles the actual execution via ``jackdaw.run()``.

        Typical notebook pattern::

            pipeline = jackdaw.WorkflowBuilder("my-pipeline")
            pipeline.add("step-a", task_a)
            pipeline.add("step-b", fn_b)

            ctx = pipeline.run(cfg.to_pipeline_input())  # executes both steps
            ctx["some_key"]                               # inspect

            pipeline.add("step-c", fn_c)
            ctx = pipeline.run(cfg.to_pipeline_input())  # a+b cache-hit, c runs
        """
        import sys
        if "IPython" not in sys.modules:
            return dict(ctx)

        if self._engine_loop is None:
            self._engine_loop = _EngineLoop()

        return self._engine_loop.run(self.build(), dict(ctx), timeout)

    def steps(self) -> list[TaskItem]:
        """Return the accumulated steps (used when nesting builders inside fork branches)."""
        return list(self._steps)

    def build(self) -> Workflow:
        """Return the assembled Workflow."""
        return Workflow(
            document=Document(
                dsl="1.0.2",
                namespace=self._namespace,
                name=self._name,
                version=self._version,
            ),
            do=list(self._steps),
        )


class WorkflowRunner:
    """Wraps DurableEngine to accept sdk-python Workflow objects directly."""

    def __init__(self) -> None:
        self._engine = DurableEngineBuilder().build()

    async def run_async(
        self,
        workflow: Workflow,
        input: dict | None = None,
        timeout: float = 30.0,
    ) -> object:
        """Execute a workflow and return the result, awaiting completion."""
        handle: ExecutionHandle = await self._engine.execute(workflow.to_yaml(), input or {})
        return await handle.wait_for_completion(timeout)

    def run(
        self,
        workflow: Workflow,
        input: dict | None = None,
        timeout: float = 30.0,
    ) -> object:
        """Execute a workflow synchronously and return the result."""
        return asyncio.run(self.run_async(workflow, input, timeout))


@runtime_checkable
class Config(Protocol):
    """Protocol for pipeline configuration objects.

    Any class with a `to_pipeline_input` method satisfies this protocol —
    no inheritance required. The method must return a flat dict that jackdaw
    passes as the workflow's initial context.
    """

    def to_pipeline_input(self) -> dict: ...


def run_from_config(
    build_fn: Callable[..., "WorkflowBuilder"],
    cfg: Config,
    timeout: float = 86400.0,
) -> object:
    """Build and run a pipeline from a config object.

    build_fn receives cfg and returns a WorkflowBuilder. cfg.to_pipeline_input()
    supplies the initial workflow context. Runs synchronously.
    """
    pipeline = build_fn(cfg).build()
    return run(pipeline, input=cfg.to_pipeline_input(), timeout=timeout)


async def run_from_config_async(
    build_fn: Callable[..., "WorkflowBuilder"],
    cfg: Config,
    timeout: float = 86400.0,
) -> object:
    """Async variant of run_from_config."""
    pipeline = build_fn(cfg).build()
    return await run_async(pipeline, input=cfg.to_pipeline_input(), timeout=timeout)


def run(workflow: Workflow, input: dict | None = None, timeout: float = 30.0) -> object:
    """Execute a workflow synchronously using a one-shot engine instance."""
    return WorkflowRunner().run(workflow, input, timeout)


async def run_async(
    workflow: Workflow,
    input: dict | None = None,
    timeout: float = 30.0,
) -> object:
    """Execute a workflow asynchronously using a one-shot engine instance."""
    return await WorkflowRunner().run_async(workflow, input, timeout)
