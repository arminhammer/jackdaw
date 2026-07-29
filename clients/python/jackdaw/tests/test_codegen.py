"""Tests for _codegen's helper-extraction: functions AND literal constants
that a step function references via module globals must be inlined into the
generated script, not just functions."""

import jackdaw
from jackdaw._codegen import _extract_helper_functions, generate_script

jackdaw.set_output_enabled(False)


NATURE_CLASSES = ["forest", "wood", "wetland"]
THRESHOLD = 5
CONFIG = {"a": 1, "b": [1, 2, 3]}


def uses_list_constant(x: int) -> dict:
    return {"y": x if x in NATURE_CLASSES else "no"}


def uses_int_constant(x: int) -> dict:
    return {"y": x > THRESHOLD}


def uses_dict_constant(x: int) -> dict:
    return {"y": CONFIG["a"]}


def helper_fn(x: int) -> int:
    return x * 2


def uses_helper_and_constant(x: int) -> dict:
    return {"y": helper_fn(x) if x in NATURE_CLASSES else 0}


def test_list_constant_is_inlined():
    helpers = _extract_helper_functions(uses_list_constant)
    assert any("NATURE_CLASSES" in h and "forest" in h for h in helpers)


def test_int_constant_is_inlined():
    helpers = _extract_helper_functions(uses_int_constant)
    assert "THRESHOLD = 5" in helpers


def test_dict_constant_is_inlined():
    helpers = _extract_helper_functions(uses_dict_constant)
    assert any("CONFIG" in h for h in helpers)


def test_function_and_constant_both_inlined_and_ordered():
    """Mixed dependencies (a helper function AND a literal constant) both
    get inlined, each before the function that uses them."""
    helpers = _extract_helper_functions(uses_helper_and_constant)
    joined = "\n".join(helpers)
    assert "def helper_fn" in joined
    assert "NATURE_CLASSES" in joined


def test_generated_script_with_list_constant_compiles_and_runs():
    """End-to-end: the exact failure mode from warm_cache.py's
    NATURE_CLASSES/PARK_CLASSES/TRAIL_CLASSES pattern — a generated script
    that references a module-level list constant must actually run, not
    NameError."""
    script = generate_script(uses_list_constant)
    compile(script, "<uses_list_constant>", "exec")

    import json
    import subprocess
    import sys

    result = subprocess.run(
        [sys.executable, "-c", script],
        input=json.dumps({"x": "forest"}),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == {"y": "forest"}


def test_session_step_referencing_module_constant_executes(tmp_path):
    """The same bug through the real Session.commit path, not just codegen."""
    from typing import TypedDict

    from pydantic import BaseModel

    class Input(BaseModel):
        x: str

    class Output(TypedDict):
        y: str

    sess = jackdaw.Session(name="const-test", input=Input(x="wood"), output=Output)
    out = sess.commit(uses_list_constant)
    assert out["y"] == "wood"