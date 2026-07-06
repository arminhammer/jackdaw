import json
import os
from pathlib import Path
from unittest.mock import patch

from serverlessworkflow.sdk.tasks import ForkTask, RunTask

from jackdaw.runner import (
    _MERGE_OUTPUT,
    _PASSTHROUGH_OUTPUT,
    WorkflowBuilder,
    _extract_module_imports,
    _imports_from_globals,
)

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _task(wf, index: int) -> RunTask:
    return wf.do[index].task


# ---------------------------------------------------------------------------
# _imports_from_source (AST strategy)
# ---------------------------------------------------------------------------


def test_source_strategy_finds_stdlib_import():
    def step(working_dir: str) -> dict:
        return {"path": os.path.join(working_dir, "out")}

    imports = _extract_module_imports(step)
    assert any("import os" in line for line in imports)


def test_source_strategy_finds_from_import():
    def step(p: str) -> dict:
        return {"resolved": str(Path(p))}

    imports = _extract_module_imports(step)
    assert any("Path" in line for line in imports)


def test_source_strategy_narrows_multi_name_import():
    # Module imports both os and json; function only uses json.
    def step(data: str) -> dict:
        return json.loads(data)

    imports = _extract_module_imports(step)
    combined = " ".join(imports)
    assert "json" in combined
    assert "import os" not in combined


def test_source_strategy_empty_for_builtins():
    def step(x: str) -> dict:
        return {"n": len(x), "u": x.upper()}

    assert _extract_module_imports(step) == []


# ---------------------------------------------------------------------------
# _imports_from_globals (dill fallback strategy)
# ---------------------------------------------------------------------------


def test_dill_fallback_triggered_when_source_unavailable():
    def step(working_dir: str) -> dict:
        return {"out": os.path.join(working_dir, "result")}

    with patch("inspect.getsource", side_effect=OSError("no source")):
        imports = _extract_module_imports(step)

    assert any("os" in line for line in imports)


def test_dill_fallback_handles_from_import():
    def step(p: str) -> dict:
        return {"out": str(Path(p))}

    with patch("inspect.getsource", side_effect=OSError("no source")):
        imports = _extract_module_imports(step)

    assert any("Path" in line for line in imports)


def test_dill_fallback_direct():
    fake_globals = {"os": os, "Path": Path}
    imports = _imports_from_globals(fake_globals, {"os", "Path"})
    combined = " ".join(imports)
    assert "os" in combined
    assert "Path" in combined


def test_dill_fallback_empty_for_missing_name():
    imports = _imports_from_globals({}, {"nonexistent"})
    assert imports == []


# ---------------------------------------------------------------------------
# WorkflowBuilder — structure
# ---------------------------------------------------------------------------


def test_build_produces_workflow():
    def greet(name: str) -> dict:
        return {"greeting": f"Hello, {name}!"}

    wf = WorkflowBuilder("test-wf").add("greet", greet).build()
    assert wf.document.name == "test-wf"
    assert len(wf.do) == 1
    assert wf.do[0].name == "greet"


def test_callable_step_reads_from_stdin():
    def step(working_dir: str, url: str) -> dict:
        return {}

    wf = WorkflowBuilder("wf").add("s", step).build()
    task = _task(wf, 0)
    assert task.run.script.stdin == "${ . }"


def test_function_source_embedded_in_code():
    def compute(n: str) -> dict:
        return {"result": int(n) * 2}

    wf = WorkflowBuilder("wf").add("compute", compute).build()
    code = _task(wf, 0).run.script.code
    assert "def compute" in code
    assert "if __name__" in code
    assert "_json.load(_sys.stdin)" in code
    assert "compute(" in code


def test_module_imports_prepended_to_code():
    def step(working_dir: str) -> dict:
        return {"out": os.path.join(working_dir, "result")}

    wf = WorkflowBuilder("wf").add("step", step).build()
    code = _task(wf, 0).run.script.code
    assert code.index("import os") < code.index("def step")


def test_output_merge_set_on_callable_steps():
    def a(x: str) -> dict:
        return {}

    def b(y: str) -> dict:
        return {}

    wf = WorkflowBuilder("wf").add("a", a).add("b", b).build()
    for item in wf.do:
        assert item.task.output.as_ == _MERGE_OUTPUT


def test_add_accepts_pretask():
    from jackdaw.runner import shell_task
    task = shell_task("mkdir", arguments=["${ .working_dir }"])
    wf = WorkflowBuilder("wf").add("make-dir", task).build()
    assert _task(wf, 0).run.shell.command == "mkdir"
    assert _task(wf, 0).output.as_ == _PASSTHROUGH_OUTPUT


def test_run_container_step():
    wf = (
        WorkflowBuilder("wf")
        .run_container(
            "extract", image="osmtools:latest", command="osmium extract -o out.pbf in.pbf"
        )
        .build()
    )
    task = _task(wf, 0)
    assert task.run.container.image == "osmtools:latest"
    assert task.output.as_ == _PASSTHROUGH_OUTPUT


def test_fork_creates_parallel_branches():
    def branch_fn(x: str) -> dict:
        return {}

    wf = (
        WorkflowBuilder("wf")
        .fork(
            "parallel",
            {
                "a": WorkflowBuilder("a").add("step-a", branch_fn),
                "b": WorkflowBuilder("b").add("step-b", branch_fn),
            },
        )
        .build()
    )
    fork_task = _task(wf, 0)
    assert isinstance(fork_task, ForkTask)
    assert {b.name for b in fork_task.fork.branches} == {"a", "b"}


def test_chaining_returns_same_builder():
    builder = WorkflowBuilder("wf")

    def fn(x: str) -> dict:
        return {}

    assert builder.add("s", fn) is builder
    assert builder.run_container("c", "img:latest", "cmd") is builder


def test_workflow_serializes_to_yaml():
    def step(value: str) -> dict:
        return {"out": value}

    wf = WorkflowBuilder("wf").add("step", step).build()
    yaml_str = wf.to_yaml()
    assert "document:" in yaml_str
    assert "language: python" in yaml_str
