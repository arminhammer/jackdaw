"""Turn Python functions into workflow script tasks.

This module owns the source-extraction pipeline: given a plain Python
function, it produces a self-contained script (imports + module-local helper
functions + the function itself + a stdin/stdout main block) wrapped in a
``RunTask``. Both the interactive ``Session`` and the ``WorkflowBuilder``
build on it.
"""

import ast
import builtins
import dis
import inspect
import textwrap
from collections.abc import Callable
from typing import Any

from serverlessworkflow.sdk.base import Input, Output, Schema
from serverlessworkflow.sdk.tasks import (
    RunConfiguration,
    RunTask,
    ScriptConfiguration,
)

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
        if ann is inspect.Parameter.empty:
            properties[name] = {}
        else:
            properties[name] = _PY_TO_JSON_SCHEMA.get(ann, {})

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


def _is_literal(val: Any) -> bool:
    """True if val is a plain literal (or nested combination) safely re-creatable via repr()."""
    if val is None or isinstance(val, (bool, int, float, str, bytes)):
        return True
    if isinstance(val, (list, tuple, set, frozenset)):
        return all(_is_literal(v) for v in val)
    if isinstance(val, dict):
        return all(_is_literal(k) and _is_literal(v) for k, v in val.items())
    return False


def _collect_helpers(
    fn: Callable,
    fn_module: Any,
    fn_globals: dict[str, Any],
    visited: set[str],
    result: list[str],
) -> None:
    """Post-order DFS: collect source for module-local helpers fn depends on —
    both functions it calls and literal constants it references (e.g. a
    module-level `NATURE_CLASSES = [...]` list).

    Processes dependencies before the functions that depend on them so the
    generated script defines helpers in the right order.
    """
    for name in _needed_global_names(fn):
        if name in visited:
            continue
        visited.add(name)
        if name not in fn_globals:
            continue
        val = fn_globals[name]
        if not callable(val):
            # Module-level constant referenced by the function: inline it as
            # a literal assignment when it's safely representable. Anything
            # else (class instances, compiled regexes, ...) can't be
            # serialized into the generated script and is left for the
            # caller to discover as a NameError, same as an unhandled import.
            if _is_literal(val):
                result.append(f"{name} = {val!r}")
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


def generate_script(fn: Callable) -> str:
    """Generate the self-contained Python script for a task function.

    The script is compiled immediately so extraction problems (unparseable
    source, decorator artifacts, notebook edge cases) surface as an in-cell
    error instead of a subprocess failure at execution time.
    """
    params = list(inspect.signature(fn).parameters.keys())
    imports = _extract_module_imports(fn)
    helpers = _extract_helper_functions(fn)
    source = textwrap.dedent(inspect.getsource(fn))
    header = "\n".join(imports) + "\n\n" if imports else ""
    helper_block = "\n\n".join(helpers) + "\n\n" if helpers else ""
    code = header + helper_block + source + _MAIN_BLOCK.format(fn_name=fn.__name__, params=params)

    try:
        compile(code, f"<jackdaw:{fn.__name__}>", "exec")
    except SyntaxError as e:
        raise ValueError(
            f"Generated script for {fn.__name__!r} does not compile "
            f"(line {e.lineno}: {e.msg}). Generated source:\n{code}"
        ) from e

    return code


def function_to_task(fn: Callable) -> RunTask:
    """Wrap a Python function as a script RunTask.

    The function receives its declared parameters by name from the accumulated
    context (via stdin JSON), and its returned dict is merged into the context
    for subsequent steps.
    """
    code = generate_script(fn)
    return RunTask(
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


def required_params(fn: Callable) -> list[str]:
    """Names of parameters without defaults — these must exist in the context."""
    return [
        name
        for name, param in inspect.signature(fn).parameters.items()
        if param.default is inspect.Parameter.empty
        and param.kind
        in (inspect.Parameter.POSITIONAL_OR_KEYWORD, inspect.Parameter.KEYWORD_ONLY)
    ]
