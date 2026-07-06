"""Tests for the Pythonic expression builder: compiled JQ + end-to-end routing."""

import asyncio
from typing import TypedDict

import pytest
from pydantic import BaseModel

import jackdaw
from jackdaw._expr import as_arg, as_output_as, as_when

jackdaw.set_output_enabled(False)


class Bbox(BaseModel):
    xmin: float
    ymin: float
    xmax: float
    ymax: float


class Input(BaseModel):
    working_dir: str
    bbox: Bbox
    combined_geojson: str | None = None


ctx = jackdaw.ref(Input)


# --------------------------------------------------------------------- #
# compilation: matches the JQ the notebooks used to write by hand
# --------------------------------------------------------------------- #


def test_path_join_compiles_like_handwritten_mkdir_arg():
    # handwritten: '${ .working_dir + "/valhalla_runs" }'
    assert as_arg(ctx.working_dir / "valhalla_runs") == '${ (.working_dir + "/valhalla_runs") }'


def test_when_comparison_compiles_bare():
    # handwritten: '.combined_geojson != null'
    assert as_when(ctx.combined_geojson != None) == "(.combined_geojson != null)"  # noqa: E711


def test_join_coerces_numbers_like_handwritten_bbox():
    expr = jackdaw.join(",", ctx.bbox.xmin, ctx.bbox.ymin)
    # handwritten: '${ (.bbox.xmin | tostring) + "," + (.bbox.ymin | tostring) }'
    assert as_arg(expr) == '${ (((.bbox.xmin | tostring) + ",") + (.bbox.ymin | tostring)) }'


def test_merge_compiles_with_input_root():
    expr = jackdaw.merge(scoped_pbf_file=ctx.working_dir / "valhalla_runs/scoped.osm.pbf")
    # handwritten: '$input + {scoped_pbf_file: ($input.working_dir + "/valhalla_runs/scoped.osm.pbf")}'
    assert (
        as_output_as(expr)
        == '$input + {scoped_pbf_file: ($input.working_dir + "/valhalla_runs/scoped.osm.pbf")}'
    )


def test_string_literals_are_json_quoted():
    assert as_when(ctx.working_dir == 'path "with" quotes') == (
        '(.working_dir == "path \\"with\\" quotes")'
    )


def test_typed_ref_rejects_unknown_fields():
    with pytest.raises(AttributeError, match="workign_dir"):
        _ = ctx.workign_dir  # typo

    with pytest.raises(AttributeError, match="nope"):
        _ = ctx.bbox.nope  # nested typo


def test_untyped_ctx_allows_ad_hoc_keys():
    assert as_when(jackdaw.ctx.anything.nested != None) == "(.anything.nested != null)"  # noqa: E711


def test_expr_has_no_truth_value():
    with pytest.raises(TypeError, match="truth value"):
        bool(ctx.working_dir == "x")


def test_raw_jq_strings_pass_through_unchanged():
    assert as_arg("${ .working_dir }") == "${ .working_dir }"
    assert as_when(".x != null") == ".x != null"


# --------------------------------------------------------------------- #
# end-to-end: expressions drive real engine behavior
# --------------------------------------------------------------------- #


class Output(TypedDict):
    dest: str


def test_exprs_route_and_compute_through_the_engine(tmp_path):
    sess = jackdaw.Session(
        name="expr-e2e",
        input=Input(
            working_dir=str(tmp_path),
            bbox=Bbox(xmin=-77.17, ymin=38.82, xmax=-77.03, ymax=38.93),
            combined_geojson=None,
        ),
        output=Output,
    )

    # Shell task with expression args: echo the joined bbox (side-effect free).
    sess.commit(
        jackdaw.shell_task(
            "echo",
            [jackdaw.join(",", ctx.bbox.xmin, ctx.bbox.ymin, ctx.bbox.xmax, ctx.bbox.ymax)],
            output_as=jackdaw.merge(dest=ctx.working_dir / "out"),
        ),
        name="compute-dest",
    )
    assert sess.ctx["dest"] == str(tmp_path) + "/out"

    # Switch on an expression condition.
    def by_polygon(working_dir: str) -> dict:
        return {"dest": working_dir + "/polygon"}

    def by_bbox(working_dir: str) -> dict:
        return {"dest": working_dir + "/bbox"}

    sess.commit(
        jackdaw.switch(
            jackdaw.case("use-polygon", by_polygon, when=(ctx.combined_geojson != None)),  # noqa: E711
            jackdaw.case("use-bbox", by_bbox),
        ),
        name="route",
    )
    assert sess.ctx["dest"].endswith("/bbox")

    # The exported artifact routes the other way with polygon input.
    yaml_doc = sess.export()

    async def run_with(input_data):
        engine = jackdaw.DurableEngineBuilder().build()
        handle = await engine.execute(yaml_doc, input_data)
        return await handle.wait_for_completion(60.0)

    polygon_input = {
        "working_dir": str(tmp_path),
        "bbox": {"xmin": -77.17, "ymin": 38.82, "xmax": -77.03, "ymax": 38.93},
        "combined_geojson": "/some/poly.geojson",
    }
    assert asyncio.run(run_with(polygon_input))["dest"].endswith("/polygon")
