import ast
import asyncio
import builtins
import dis
import inspect
import textwrap
from collections.abc import Callable, Generator
from typing import Any, Protocol, runtime_checkable

from serverlessworkflow.sdk import Workflow
from serverlessworkflow.sdk.base import Input, Output, Schema, TaskItem
from serverlessworkflow.sdk.tasks import (
    ContainerConfiguration,
    DoTask,
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


def call_step(fn: Callable, ctx: dict, **overrides: Any) -> dict:
    """Call a pipeline step function directly and return the updated context.

    In a Jupyter/IPython session: selects parameters from ctx by name, applies
    any overrides, calls fn, and merges the result dict back into ctx.

    Outside an interactive session (plain ``python script.py``): returns ctx
    unchanged so that call_step cells in a notebook file are no-ops when the
    file is executed as a script — the __main__ block handles execution instead.

    Example notebook pattern::

        ctx = cfg.to_pipeline_input()

        def filter_feeds(gtfs_csv_file, bbox, gtfs_ignore_list) -> dict: ...
        ctx = jackdaw.call_step(filter_feeds, ctx)   # no-op when run as script
        # ctx["download_urls"]  # inspect / narrow

        def download_feeds(working_dir, download_urls) -> dict: ...
        ctx = jackdaw.call_step(download_feeds, ctx)

        if __name__ == "__main__":
            jackdaw.run(build_pipeline(cfg).build(), input=cfg.to_pipeline_input())
    """
    import sys
    if "IPython" not in sys.modules:
        return dict(ctx)
    sig = inspect.signature(fn)
    kwargs: dict[str, Any] = {}
    for name, param in sig.parameters.items():
        if name in overrides:
            kwargs[name] = overrides[name]
        elif name in ctx:
            kwargs[name] = ctx[name]
        elif param.default is not inspect.Parameter.empty:
            kwargs[name] = param.default
    result = fn(**kwargs)
    if result is None:
        return dict(ctx)
    merged = dict(ctx)
    merged.update(result)
    return merged


class WorkflowBuilder:
    """Fluent builder that assembles a Workflow from Python functions, shell commands, and containers."""

    def __init__(self, name: str = "", namespace: str = "default", version: str = "0.1.0") -> None:
        self._name = name
        self._namespace = namespace
        self._version = version
        self._steps: list[TaskItem] = []
        # Tracks original callables for debug_run. None for shell/container steps.
        self._step_fns: list[tuple[str, Callable | None]] = []

    def step(
        self,
        name: str,
        fn: Callable,
    ) -> "WorkflowBuilder":
        """Add a Python function as a sequential step.

        This is the default step type — no language qualifier needed.

        Module-level imports are detected and prepended automatically. The function
        receives its declared parameters by name from the accumulated context via stdin
        JSON (not positional argv), so types other than str are preserved. Its return
        dict is merged into the context for subsequent steps.

        input.schema is generated from the function's type annotations.
        input.from_ scopes the task input to exactly the declared parameters.
        """
        params = list(inspect.signature(fn).parameters.keys())
        imports = _extract_module_imports(fn)
        helpers = _extract_helper_functions(fn)
        source = textwrap.dedent(inspect.getsource(fn))
        header = "\n".join(imports) + "\n\n" if imports else ""
        helper_block = "\n\n".join(helpers) + "\n\n" if helpers else ""
        code = header + helper_block + source + _MAIN_BLOCK.format(fn_name=fn.__name__, params=params)

        task = RunTask(
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
        self._step_fns.append((name, fn))
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
        self._step_fns.append((name, None))
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
        self._step_fns.append((name, None))
        return self

    def fork(self, name: str, branches: dict[str, "WorkflowBuilder"]) -> "WorkflowBuilder":
        """Add parallel branches. Each value is a WorkflowBuilder defining that branch."""
        branch_items = [
            TaskItem(name=branch_name, task=DoTask(do=sub.steps()))
            for branch_name, sub in branches.items()
        ]
        self._steps.append(TaskItem(name=name, task=ForkTask(fork=ForkConfiguration(branches=branch_items))))
        self._step_fns.append((name, None))
        return self

    def debug_run(self, input: dict | None = None) -> "Generator[tuple[str, dict], dict | None, None]":
        """Step through the pipeline one task at a time, yielding after each Python step.

        Yields ``(step_name, ctx)`` after each step completes. The caller may
        inspect or modify ``ctx`` and inject it back via ``send(modified_ctx)``
        before the next step runs.  Sending ``None`` (or calling ``next()``)
        continues with the unmodified context.

        Shell and container steps have no Python callable — they are skipped in
        debug mode and their names are reported to stderr so you know they were
        bypassed. Run the full pipeline via ``jackdaw.run()`` to execute them.

        Example::

            gen = pipeline.debug_run(input=cfg.to_pipeline_input())
            step, ctx = next(gen)           # runs first Python step
            ctx["download_urls"]            # inspect
            ctx["download_urls"] = ctx["download_urls"][:2]
            step, ctx = gen.send(ctx)       # inject modified ctx, run next step
        """
        import sys
        ctx: dict = dict(input or {})
        for step_name, fn in self._step_fns:
            if fn is None:
                print(f"[debug_run] skipping non-Python step: {step_name!r}", file=sys.stderr)
                new_ctx: dict | None = yield (step_name, ctx)
            else:
                ctx = call_step(fn, ctx)
                new_ctx = yield (step_name, ctx)
            if new_ctx is not None:
                ctx = new_ctx

    def add(self, name: str, task: RunTask) -> "WorkflowBuilder":
        """Add a pre-built RunTask directly — the entry point for reusable step factories."""
        self._steps.append(TaskItem(name=name, task=task))
        self._step_fns.append((name, None))
        return self

    # Deprecated alias — prefer step()
    def run_python(self, name: str, fn: Callable, args: list[str] | None = None) -> "WorkflowBuilder":
        return self.step(name, fn)

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


class Session:
    """Interactive pipeline session for notebook-driven workflow development.

    Add steps one at a time and execute them immediately through the jackdaw
    engine.  The accumulated context is updated after each step and available
    for inspection.  When you are happy with the result, export the assembled
    workflow for production use.

    Works for all task types — Python functions, shell commands, containers,
    forks, and for-loops.  The only exception is ``wait`` tasks (they block on
    external events and do not fit the synchronous cell-by-cell model).

    Example::

        session = jackdaw.Session(input=cfg.to_pipeline_input())

        def filter_feeds(gtfs_csv_file, bbox, gtfs_ignore_list) -> dict: ...
        ctx = session.run("filter-feeds", filter_feeds)
        ctx["download_urls"]                          # inspect
        ctx["download_urls"] = ctx["download_urls"][:2]
        session.ctx = ctx                             # inject narrowed list

        ctx = session.run("download-feeds", download_feeds)
        ctx = session.run("make-dir", mkdir("${ .working_dir }"))

        # Export when done
        pipeline = session.export("gtfs-pipeline", namespace="data-pipelines")
        jackdaw.run(pipeline.build(), input=cfg.to_pipeline_input())
    """

    def __init__(
        self,
        input: dict | None = None,
        name: str = "session",
        namespace: str = "default",
        version: str = "0.1.0",
    ) -> None:
        from ._jackdaw import Session as _RustSession

        self._session = _RustSession(input or {})
        self._name = name
        self._namespace = namespace
        self._version = version
        # Kept for WorkflowBuilder-based export (preserves original callables).
        self._step_fns: list[tuple[str, Any]] = []

    @property
    def ctx(self) -> dict:
        """The current accumulated context dict."""
        return dict(self._session.ctx)

    @ctx.setter
    def ctx(self, value: dict) -> None:
        self._session.ctx = value

    def run(self, name: str, fn_or_task: Any, timeout: float = 60.0) -> dict:
        """Add a step to the session, executing it when in an interactive session.

        fn_or_task — a plain Python callable *or* a pre-built ``RunTask``
            (from ``shell_task``, ``container_task``, ``mkdir``, etc.).
            For forks and for-loops pass a ``WorkflowBuilder`` whose sole
            entry is the compound task.

        **In a Jupyter/IPython session**: the step runs immediately through the
        jackdaw engine and the updated context dict is returned so you can
        inspect it in the next cell.

        **Outside an interactive session** (``python script.py``): the step
        definition is accumulated via ``add`` but not executed — ``__main__``
        runs the full assembled workflow via ``jackdaw.run(session.export().build())``.

        Returns the current context dict (updated after execution in
        interactive mode, unchanged in script mode).
        """
        import sys

        step_builder = WorkflowBuilder("_step", namespace="interactive")
        if callable(fn_or_task):
            step_builder.step(name, fn_or_task)
        else:
            step_builder.add(name, fn_or_task)
        self._step_fns.append((name, fn_or_task))

        workflow_yaml = step_builder.build().to_yaml()
        if "IPython" in sys.modules:
            result = self._session.run(workflow_yaml, timeout)
            return dict(result)

        self._session.add(workflow_yaml)
        return dict(self._session.ctx)

    def export(
        self,
        name: str | None = None,
        namespace: str | None = None,
        version: str | None = None,
    ) -> "WorkflowBuilder":
        """Return a ``WorkflowBuilder`` with all executed steps.

        The builder can be used directly with ``jackdaw.run()`` or further
        extended before running.
        """
        builder = WorkflowBuilder(
            name or self._name,
            namespace=namespace or self._namespace,
            version=version or self._version,
        )
        for step_name, fn_or_task in self._step_fns:
            if callable(fn_or_task):
                builder.step(step_name, fn_or_task)
            else:
                builder.add(step_name, fn_or_task)
        return builder

    def export_yaml(
        self,
        name: str | None = None,
        namespace: str | None = None,
        version: str | None = None,
    ) -> str:
        """Return the assembled workflow as a YAML string.

        Built from native Rust ``TaskDefinition`` structs — language-agnostic
        and suitable for use with ``jackdaw run`` or any other client.
        """
        return self._session.export_yaml(
            name or self._name,
            namespace or self._namespace,
            version or self._version,
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
