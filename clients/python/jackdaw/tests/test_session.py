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
# on-disk persistence and cache (durable_db / cache_db)
# --------------------------------------------------------------------- #


def test_durable_db_and_cache_db_create_files(tmp_path):
    durable_db = tmp_path / "durable.db"
    cache_db = tmp_path / "cache.db"
    sess = make_session(durable_db=str(durable_db), cache_db=str(cache_db))
    sess.commit(double)

    assert durable_db.exists()
    assert cache_db.exists()
    assert cache_db.stat().st_size > 0


def test_cache_db_survives_across_separate_sessions(tmp_path):
    """A step committed on one Session instance is a cache hit on a brand
    new Session pointed at the same cache_db — proving the cache is real
    on-disk state, not per-process memory.

    The step runs in a subprocess via generated source, so it can't close
    over a Python-side counter — it counts its own executions by appending
    to a file, a real external side effect only a real re-run would repeat.
    """
    cache_db = str(tmp_path / "cache.db")
    call_log = str(tmp_path / "calls.log")

    def counted(x: int, call_log: str) -> dict:
        with open(call_log, "a") as f:
            f.write("call\n")
        return {"y": x * 2}

    sess_a = make_session(cache_db=cache_db)
    sess_a.update(call_log=call_log)
    sess_a.commit(counted, name="counted")
    assert open(call_log).read().count("call\n") == 1

    # redb holds an exclusive file lock while open, so the first session must
    # release it before a second one opens the same file — exactly why the
    # real usage pattern is "reopen in a new process" (kernel restart), not
    # two live handles at once.
    del sess_a

    # Fresh Session, fresh in-process engine, same on-disk cache file, same
    # step name/definition/input: must be a cache hit, not a re-execution.
    sess_b = make_session(cache_db=cache_db)
    sess_b.update(call_log=call_log)
    sess_b.commit(counted, name="counted")
    assert open(call_log).read().count("call\n") == 1, (
        "step should not re-execute: same cache_db, same input"
    )


def test_without_cache_db_each_session_re_executes(tmp_path):
    """Baseline: without cache_db, a fresh Session has no memory of prior
    runs, so the same step executes again."""
    call_log = str(tmp_path / "calls.log")

    def counted(x: int, call_log: str) -> dict:
        with open(call_log, "a") as f:
            f.write("call\n")
        return {"y": x * 2}

    s1 = make_session()
    s1.update(call_log=call_log)
    s1.commit(counted, name="counted")

    s2 = make_session()
    s2.update(call_log=call_log)
    s2.commit(counted, name="counted")

    assert open(call_log).read().count("call\n") == 2


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


# --------------------------------------------------------------------- #
# foreach: iteration steps
# --------------------------------------------------------------------- #


class LoopInput(BaseModel):
    items: list[int]
    total_offset: int


def test_foreach_iterates_and_accumulates():
    sess = jackdaw.Session(
        name="loopy",
        input=LoopInput(items=[1, 2, 3], total_offset=100),
        output=PipelineOutput,
    )

    def per_item(item: int, total_offset: int) -> dict:
        # `item` is the loop variable injected by the engine each iteration.
        return {"y": total_offset + item}

    ctx_after = sess.commit(
        jackdaw.foreach("item", jackdaw.ctx.items, per_item), name="per-item"
    )
    # Iteration outputs merge into the context; the last item wins for `y`.
    assert ctx_after["y"] == 103

    yaml_doc = sess._prepare(jackdaw.foreach("item", jackdaw.ctx.items, per_item), "per-item")
    assert "for:" in yaml_doc and "each: item" in yaml_doc and "in: .items" in yaml_doc


def test_foreach_body_may_use_loop_variable_not_in_context():
    """The loop variable is injected per-iteration; commit-time validation
    must not demand it from the seed context."""
    sess = jackdaw.Session(
        name="loopy",
        input=LoopInput(items=[5], total_offset=0),
        output=PipelineOutput,
    )

    def body(item: int) -> dict:
        return {"y": item}

    ctx_after = sess.commit(jackdaw.foreach("item", ".items", body), name="loop")
    assert ctx_after["y"] == 5


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


# --------------------------------------------------------------------- #
# subworkflow: composing sessions from already-exported artifacts
# --------------------------------------------------------------------- #


class SubInput(BaseModel):
    a: int


class SubOutput(TypedDict):
    b: int


def double_a(a: int) -> dict:
    return {"b": a * 2}


def _export_sub_pipeline(tmp_path) -> str:
    sub = jackdaw.Session(
        name="sub-pipeline",
        namespace="test-ns",
        version="1.0.0",
        input=SubInput(a=1),
        output=SubOutput,
    )
    sub.commit(double_a)
    path = tmp_path / "sub.sw.yaml"
    sub.export(str(path))
    return str(path)


class CallerInput(BaseModel):
    x: int


class CallerOutput(TypedDict):
    b: int


def test_subworkflow_calls_exported_artifact_and_merges_output(tmp_path):
    sub_path = _export_sub_pipeline(tmp_path)

    caller = jackdaw.Session(
        name="caller",
        input=CallerInput(x=21),
        output=CallerOutput,
        registry=[sub_path],
    )
    ctx = caller.commit(
        jackdaw.subworkflow(
            namespace="test-ns", name="sub-pipeline", version="1.0.0",
            input={"a": jackdaw.ctx.x},
        ),
        name="call-sub",
    )
    assert ctx["b"] == 42


def test_subworkflow_enforces_its_own_input_schema(tmp_path):
    """The sub-workflow's contract is enforced at the call boundary,
    independently of the caller's own contract."""
    sub_path = _export_sub_pipeline(tmp_path)

    caller = jackdaw.Session(
        name="caller",
        input=CallerInput(x=21),
        output=CallerOutput,
        registry=[sub_path],
    )
    with pytest.raises(RuntimeError, match="schema validation"):
        caller.commit(
            jackdaw.subworkflow(
                namespace="test-ns", name="sub-pipeline", version="1.0.0",
                input={"a": jackdaw.ctx.x != jackdaw.ctx.x},  # bool, not int
            ),
            name="call-sub",
        )


def test_unregistered_subworkflow_fails_clearly():
    caller = jackdaw.Session(name="caller", input=CallerInput(x=21), output=CallerOutput)
    with pytest.raises(RuntimeError, match="not found|not registered|Unknown workflow"):
        caller.commit(
            jackdaw.subworkflow(namespace="test-ns", name="sub-pipeline", version="1.0.0"),
            name="call-sub",
        )


def test_subworkflow_requires_name():
    caller = jackdaw.Session(name="caller", input=CallerInput(x=21), output=CallerOutput)
    with pytest.raises(ValueError, match="name"):
        caller.commit(jackdaw.subworkflow(namespace="test-ns", name="sub-pipeline"))
