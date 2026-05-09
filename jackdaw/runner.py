import ast
import asyncio
import builtins
import dis
import inspect
import textwrap
from collections.abc import Callable
from typing import Any

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
    output_as: str | None = None,
) -> RunTask:
    """Standalone factory for a reusable container step.

    Volume keys and values may be JQ expressions evaluated against the context at runtime:

        valhalla_container = container_task(
            image="ghcr.io/valhalla/valhalla:run-latest",
            command="valhalla_build_tiles -c /data/valhalla.json /data/scoped.osm.pbf",
            volumes={"${ .working_dir }": "/data"},
        )
    """
    return RunTask(
        run=RunConfiguration(
            container=ContainerConfiguration(
                image=image,
                command=command,
                arguments=arguments,
                environment=environment,
                volumes=volumes,
            ),
        ),
        output=Output(as_=output_as or _PASSTHROUGH_OUTPUT),
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


class WorkflowBuilder:
    """Fluent builder that assembles a Workflow from Python functions, shell commands, and containers."""

    def __init__(self, name: str = "", namespace: str = "default", version: str = "0.1.0") -> None:
        self._name = name
        self._namespace = namespace
        self._version = version
        self._steps: list[TaskItem] = []

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
        source = textwrap.dedent(inspect.getsource(fn))
        header = "\n".join(imports) + "\n\n" if imports else ""
        code = header + source + _MAIN_BLOCK.format(fn_name=fn.__name__, params=params)

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
        output_as: str | None = None,
    ) -> "WorkflowBuilder":
        """Add a container step.

        command  — shell script run inside the container via sh -c (supports redirects).
        arguments — positional $1/$2/… args passed after the command.
        volumes  — host→container path mappings; keys/values may be JQ expressions,
                   e.g. volumes={"${ .working_dir }": "/data"}.
        output_as — JQ expression for the output transform; defaults to passthrough.
        """
        task = container_task(
            image=image,
            command=command,
            arguments=arguments,
            environment=environment,
            volumes=volumes,
            output_as=output_as,
        )
        self._steps.append(TaskItem(name=name, task=task))
        return self

    def fork(self, name: str, branches: dict[str, "WorkflowBuilder"]) -> "WorkflowBuilder":
        """Add parallel branches. Each value is a WorkflowBuilder defining that branch."""
        branch_items = [
            TaskItem(name=branch_name, task=DoTask(do=sub.steps()))
            for branch_name, sub in branches.items()
        ]
        self._steps.append(TaskItem(name=name, task=ForkTask(fork=ForkConfiguration(branches=branch_items))))
        return self

    def add(self, name: str, task: RunTask) -> "WorkflowBuilder":
        """Add a pre-built RunTask directly — the entry point for reusable step factories."""
        self._steps.append(TaskItem(name=name, task=task))
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
