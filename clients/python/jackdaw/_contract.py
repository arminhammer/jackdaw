"""Derive a workflow's typed input/output contract.

The workflow input is a *typed instance* — a pydantic model instance or a
dataclass instance — which carries both the seed values and the type that
compiles to the input JSON Schema. The workflow output is a *type* (a
TypedDict, pydantic model, or dataclass) describing the keys the workflow
guarantees in its output:

    class GtfsInput(BaseModel):
        working_dir: str
        gtfs_url: str

    class GtfsOutput(TypedDict):
        gtfs_feeds_dir: str

    sess = jackdaw.Session(
        name="gtfs-pipeline",
        input=GtfsInput(working_dir="/tmp/data", gtfs_url="..."),
        output=GtfsOutput,
    )
"""

import dataclasses
from typing import Any

from pydantic import TypeAdapter


class ContractError(TypeError):
    """The workflow input/output contract is not fully typed."""


def input_contract(instance: Any) -> tuple[dict, dict]:
    """Compile a typed input instance into (seed values, input JSON Schema).

    Accepts a pydantic model instance or a dataclass instance. The instance's
    class defines the schema; the instance's fields are the seed values.
    """
    if isinstance(instance, type):
        raise ContractError(
            "Session input must be a typed *instance* carrying the seed "
            f"values, not a class: got {instance!r}. "
            "Construct it, e.g. input=MyInput(working_dir='/tmp', ...)"
        )

    if hasattr(instance, "model_dump") and hasattr(type(instance), "model_json_schema"):
        # pydantic BaseModel instance
        values = instance.model_dump()
        schema = type(instance).model_json_schema()
    elif dataclasses.is_dataclass(instance):
        values = dataclasses.asdict(instance)
        schema = TypeAdapter(type(instance)).json_schema()
    else:
        raise ContractError(
            "Session input must be a typed instance (a pydantic model or a "
            f"dataclass), got {type(instance).__name__!r}. Plain dicts carry "
            "no type information — define an input model:\n"
            "    class MyInput(BaseModel):\n"
            "        working_dir: str\n"
            "and pass input=MyInput(working_dir=...)"
        )

    # The context legitimately accumulates keys beyond the declared inputs;
    # the contract is "at least these keys, with these types".
    schema.pop("additionalProperties", None)
    return values, schema


def output_contract(tp: Any) -> dict:
    """Compile a typed output declaration (TypedDict, pydantic model, or
    dataclass *type*) into a JSON Schema."""
    if not isinstance(tp, type):
        raise ContractError(
            "Session output must be a type describing the workflow's output "
            f"(a TypedDict, pydantic model, or dataclass), got {tp!r}"
        )
    try:
        return TypeAdapter(tp).json_schema()
    except Exception as e:
        raise ContractError(
            f"Cannot derive a JSON Schema from output type {tp!r}: {e}"
        ) from e
