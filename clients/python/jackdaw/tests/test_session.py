"""Tests for the interactive Session: typed contracts, preview/commit/replay/rollback/export."""

import asyncio
from dataclasses import dataclass
from typing import TypedDict

import pytest
from pydantic import BaseModel

import jackdaw
from jackdaw._contract import ContractError

jackdaw.set_output_enabled(False)


class PipelineInput(BaseModel):
    x: int


class PipelineOutput(TypedDict):
    y: int
    z: int


def double(x: int) -> dict:
    return {"y": x * 2}


def add_one(y: int) -> dict:
    return {"z": y + 1}


def make_session(**kwargs) -> jackdaw.Session:
    return jackdaw.Session(
        name="pipeline", input=PipelineInput(x=2), output=PipelineOutput, **kwargs
    )


# --------------------------------------------------------------------- #
# typed contracts
# --------------------------------------------------------------------- #


def test_input_instance_seeds_values_and_schema():
    sess = make_session()
    assert sess.ctx == {"x": 2}


def test_dataclass_input_is_supported():
    @dataclass
    class DcInput:
        x: int

    sess = jackdaw.Session(name="dc", input=DcInput(x=3), output=PipelineOutput)
    assert sess.ctx == {"x": 3}


def test_input_class_instead_of_instance_is_rejected():
    with pytest.raises(ContractError, match="instance"):
        jackdaw.Session(name="t", input=PipelineInput, output=PipelineOutput)


def test_plain_dict_input_is_rejected():
    with pytest.raises(ContractError, match="typed instance"):
        jackdaw.Session(name="t", input={"x": 2}, output=PipelineOutput)


def test_output_must_be_a_type():
    with pytest.raises(ContractError, match="type"):
        jackdaw.Session(name="t", input=PipelineInput(x=1), output={"y": int})


def test_exported_workflow_declares_both_schemas():
    sess = make_session()
    sess.commit(double)
    sess.commit(add_one)
    yaml_doc = sess.export()

    assert "input:" in yaml_doc and "output:" in yaml_doc
    assert "schema:" in yaml_doc
    # Input contract: x required; output contract: y and z guaranteed.
    assert "x" in yaml_doc and "z" in yaml_doc


def test_exported_artifact_enforces_input_schema():
    sess = make_session()
    sess.commit(double)
    sess.commit(add_one)
    yaml_doc = sess.export()

    async def run_with(input_data):
        engine = jackdaw.DurableEngineBuilder().build()
        handle = await engine.execute(yaml_doc, input_data)
        return await handle.wait_for_completion(60.0)

    # Conforming input runs.
    result = asyncio.run(run_with({"x": 3}))
    assert result["y"] == 6 and result["z"] == 7

    # Non-conforming input is rejected before execution.
    with pytest.raises(RuntimeError, match="schema validation"):
        asyncio.run(run_with({"x": "wrong-type"}))


# --------------------------------------------------------------------- #
# preview / commit / replay / rollback
# --------------------------------------------------------------------- #


def test_preview_does_not_mutate_session():
    sess = make_session()

    out = sess.preview(double)
    assert out["y"] == 4
    assert sess.ctx == {"x": 2}
    assert sess.status() == []

    # Idempotent: previewing again gives the same candidate.
    assert sess.preview(double) == out


def test_commit_records_and_advances_context():
    sess = make_session()

    ctx = sess.commit(double)
    assert ctx["x"] == 2 and ctx["y"] == 4

    ctx = sess.commit(add_one)
    assert ctx["z"] == 5
    assert sess.status() == [
        {"name": "double", "stale": False},
        {"name": "add-one", "stale": False},
    ]


def test_recommit_replaces_step_and_marks_downstream_stale():
    sess = make_session()
    sess.commit(double)
    sess.commit(add_one)

    def double_v2(x: int) -> dict:
        return {"y": x * 10}

    # Same step name → upsert: rewind, replace, downstream stale.
    ctx = sess.commit(double_v2, name="double")
    assert ctx["y"] == 20
    assert "z" not in ctx
    assert sess.status() == [
        {"name": "double", "stale": False},
        {"name": "add-one", "stale": True},
    ]

    ctx = sess.replay()
    assert ctx["y"] == 20 and ctx["z"] == 21
    assert all(not s["stale"] for s in sess.status())


def test_export_refuses_stale_steps():
    sess = make_session()
    sess.commit(double)
    sess.commit(add_one)

    def double_v2(x: int) -> dict:
        return {"y": x * 10}

    sess.commit(double_v2, name="double")
    with pytest.raises(RuntimeError, match="add-one"):
        sess.export()


def test_exported_artifact_runs_fresh(tmp_path):
    sess = make_session()
    sess.commit(double)
    sess.commit(add_one)

    out_file = tmp_path / "pipeline.sw.yaml"
    yaml_doc = sess.export(str(out_file))
    assert out_file.read_text() == yaml_doc

    async def run_artifact():
        engine = jackdaw.DurableEngineBuilder().build()
        handle = await engine.execute(yaml_doc, {"x": 3})
        return await handle.wait_for_completion(60.0)

    result = asyncio.run(run_artifact())
    assert result["y"] == 6 and result["z"] == 7


def test_rollback_restores_context_and_truncates():
    sess = make_session()
    sess.commit(double)
    sess.commit(add_one)

    ctx = sess.rollback("add-one")
    assert "z" not in ctx and ctx["y"] == 4
    assert [s["name"] for s in sess.status()] == ["double"]


def test_update_merges_into_context():
    sess = make_session()
    ctx = sess.update(extra=42)
    assert ctx == {"x": 2, "extra": 42}


def test_missing_required_params_fail_before_execution():
    sess = make_session()

    def needs_missing(nonexistent_key: str) -> dict:
        return {}

    with pytest.raises(ValueError, match="nonexistent_key"):
        sess.preview(needs_missing)


def test_typed_task_objects_commit_with_explicit_name():
    sess = make_session()
    task = jackdaw.shell_task("true")
    assert isinstance(task, jackdaw.RunShellTask)

    with pytest.raises(ValueError, match="name"):
        sess.commit(task)

    sess.commit(task, name="noop")
    assert [s["name"] for s in sess.status()] == ["noop"]


def test_untyped_steps_are_rejected():
    sess = make_session()
    with pytest.raises(TypeError, match="typed"):
        sess.commit({"not": "a step"}, name="nope")


# --------------------------------------------------------------------- #
# switch: runtime conditional steps
# --------------------------------------------------------------------- #


class SwitchInput(BaseModel):
    x: int
    polygon: str | None = None


def _switch_session(polygon: str | None) -> jackdaw.Session:
    return jackdaw.Session(
        name="switchy",
        input=SwitchInput(x=1, polygon=polygon),
        output=PipelineOutput,
    )


def by_polygon(x: int) -> dict:
    return {"y": x, "z": 100}


def by_bbox(x: int) -> dict:
    return {"y": x, "z": 200}


def make_extract_switch() -> jackdaw.Switch:
    return jackdaw.switch(
        jackdaw.case("use-polygon", by_polygon, when=".polygon != null"),
        jackdaw.case("use-bbox", by_bbox),  # default
    )


def test_switch_routes_at_runtime_in_session():
    sess = _switch_session(polygon="/tmp/x.geojson")
    ctx = sess.commit(make_extract_switch(), name="extract")
    assert ctx["z"] == 100

    sess = _switch_session(polygon=None)
    ctx = sess.commit(make_extract_switch(), name="extract")
    assert ctx["z"] == 200


def test_switch_requires_name():
    sess = _switch_session(polygon=None)
    with pytest.raises(ValueError, match="name"):
        sess.commit(make_extract_switch())


def test_exported_switch_branches_on_input():
    """The exported artifact carries BOTH branches; the input decides."""
    sess = _switch_session(polygon=None)
    sess.commit(make_extract_switch(), name="extract")
    yaml_doc = sess.export()
    assert "switch:" in yaml_doc

    async def run_with(input_data):
        engine = jackdaw.DurableEngineBuilder().build()
        handle = await engine.execute(yaml_doc, input_data)
        return await handle.wait_for_completion(60.0)

    assert asyncio.run(run_with({"x": 5, "polygon": "/p.geojson"}))["z"] == 100
    assert asyncio.run(run_with({"x": 5, "polygon": None}))["z"] == 200
