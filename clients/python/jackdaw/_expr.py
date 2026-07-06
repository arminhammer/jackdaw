"""Pythonic expression builder that compiles to JQ.

Authoring-time sugar only: expressions built here compile to the same JQ
strings you could write by hand, so the exported workflow YAML stays plain
spec. The engine evaluates the JQ; nothing here executes.

    ctx = jackdaw.ref(OsmInput)      # typed: fields checked against the model

    mkdir(ctx.working_dir / "valhalla_runs")
    #  -> '${ .working_dir + "/valhalla_runs" }'

    when=(ctx.combined_geojson != None)
    #  -> '.combined_geojson != null'

    jackdaw.join(",", ctx.bbox.xmin, ctx.bbox.ymin)
    #  -> '${ (.bbox.xmin | tostring) + "," + (.bbox.ymin | tostring) }'

    output_as=jackdaw.merge(scoped_pbf_file=ctx.working_dir / "out.pbf")
    #  -> '$input + {scoped_pbf_file: ($input.working_dir + "/out.pbf")}'

Compilation is context-aware, so callers never learn the spec's quirks:
task arguments get the ``${ ... }`` wrapper, `when:`/`for.in:` are emitted
bare, and inside `output.as` the context root is ``$input`` instead of ``.``.
"""

import json
from typing import Any


class Expr:
    """A JQ expression under construction. Immutable; operators build new nodes."""

    # -- operator overloads ------------------------------------------------ #

    def __truediv__(self, other: "Expr | str") -> "Expr":
        """Path-join with "/": ctx.working_dir / "valhalla_runs"."""
        if isinstance(other, str):
            return _BinOp("+", self, _Lit("/" + other))
        return _BinOp("+", _BinOp("+", self, _Lit("/")), _tostring(other))

    def __add__(self, other: Any) -> "Expr":
        return _BinOp("+", self, _lift(other))

    def __radd__(self, other: Any) -> "Expr":
        return _BinOp("+", _lift(other), self)

    def __sub__(self, other: Any) -> "Expr":
        return _BinOp("-", self, _lift(other))

    def __mul__(self, other: Any) -> "Expr":
        return _BinOp("*", self, _lift(other))

    def __eq__(self, other: Any) -> "Expr":  # type: ignore[override]
        return _BinOp("==", self, _lift(other))

    def __ne__(self, other: Any) -> "Expr":  # type: ignore[override]
        return _BinOp("!=", self, _lift(other))

    def __gt__(self, other: Any) -> "Expr":
        return _BinOp(">", self, _lift(other))

    def __ge__(self, other: Any) -> "Expr":
        return _BinOp(">=", self, _lift(other))

    def __lt__(self, other: Any) -> "Expr":
        return _BinOp("<", self, _lift(other))

    def __le__(self, other: Any) -> "Expr":
        return _BinOp("<=", self, _lift(other))

    __hash__ = object.__hash__

    def __bool__(self) -> bool:
        raise TypeError(
            "An Expr has no truth value at authoring time — it evaluates in "
            "the engine at runtime. Use it as a `when=` condition, or compare "
            "with `is None` / `is not None` if you meant the Python object."
        )

    def tostring(self) -> "Expr":
        """Explicit JQ tostring coercion: (expr | tostring)."""
        return _tostring(self)

    # -- compilation -------------------------------------------------------- #

    def _compile(self, root: str) -> str:
        raise NotImplementedError

    def __repr__(self) -> str:
        return f"Expr({self._compile('.')!r})"


class _Field(Expr):
    """A context field reference, e.g. .bbox.xmin."""

    def __init__(self, path: tuple[str, ...], model: Any) -> None:
        object.__setattr__(self, "_fpath", path)
        object.__setattr__(self, "_model", model)

    def __getattr__(self, name: str) -> Expr:
        if name.startswith("_"):
            raise AttributeError(name)
        model = object.__getattribute__(self, "_model")
        path = object.__getattribute__(self, "_fpath")
        if model is not None:
            fields = getattr(model, "model_fields", None)
            if fields is None or name not in fields:
                known = ", ".join(sorted(fields)) if fields else "(none)"
                raise AttributeError(
                    f"{model.__name__} has no field {name!r} (fields: {known})"
                )
            sub = fields[name].annotation
            sub_model = sub if hasattr(sub, "model_fields") else None
            return _Field(path + (name,), model=sub_model)
        return _Field(path + (name,), model=None)

    def __getitem__(self, key: str) -> Expr:
        path = object.__getattribute__(self, "_fpath")
        return _Field(path + (key,), model=None)

    def _compile(self, root: str) -> str:
        dotted = "".join(f".{seg}" for seg in object.__getattribute__(self, "_fpath"))
        return dotted if root == "." else f"{root}{dotted}"


class _Lit(Expr):
    """A literal value, JSON-encoded (proper quoting, None -> null)."""

    def __init__(self, value: Any) -> None:
        object.__setattr__(self, "_value", value)

    def _compile(self, root: str) -> str:
        return json.dumps(object.__getattribute__(self, "_value"))


class _BinOp(Expr):
    def __init__(self, op: str, left: Expr, right: Expr) -> None:
        object.__setattr__(self, "_op", op)
        object.__setattr__(self, "_left", left)
        object.__setattr__(self, "_right", right)

    def _compile(self, root: str) -> str:
        op = object.__getattribute__(self, "_op")
        left = object.__getattribute__(self, "_left")._compile(root)
        right = object.__getattribute__(self, "_right")._compile(root)
        return f"({left} {op} {right})"


class _Pipe(Expr):
    """expr | filter, e.g. (.bbox.xmin | tostring)."""

    def __init__(self, inner: Expr, jq_filter: str) -> None:
        object.__setattr__(self, "_inner", inner)
        object.__setattr__(self, "_filter", jq_filter)

    def _compile(self, root: str) -> str:
        inner = object.__getattribute__(self, "_inner")._compile(root)
        return f"({inner} | {object.__getattribute__(self, '_filter')})"


class Merge(Expr):
    """An output merge: $input + {key: value, ...}. Build with merge()."""

    def __init__(self, fields: dict[str, Expr]) -> None:
        object.__setattr__(self, "_fields", fields)

    def _compile(self, root: str) -> str:
        fields = object.__getattribute__(self, "_fields")
        body = ", ".join(f"{k}: {v._compile(root)}" for k, v in fields.items())
        return f"$input + {{{body}}}"


def _lift(value: Any) -> Expr:
    return value if isinstance(value, Expr) else _Lit(value)


def _tostring(e: Expr) -> Expr:
    # Literals JSON-encode to strings already; only field/computed values
    # need runtime coercion.
    return _Pipe(e, "tostring") if not isinstance(e, _Lit) else e


# --------------------------------------------------------------------- #
# public constructors
# --------------------------------------------------------------------- #


def ref(model: type | None = None) -> Expr:
    """A typed context root: field access is validated against `model`
    (a pydantic model class), so typos fail in the cell with the field list.
    Call without a model for an unchecked root (`jackdaw.ctx` is one)."""
    if model is not None and not hasattr(model, "model_fields"):
        raise TypeError(
            f"ref() takes a pydantic model class (got {model!r}); "
            "use jackdaw.ctx for an unchecked context root"
        )
    return _Field((), model=model)


# Unchecked context root for ad-hoc keys produced mid-pipeline.
ctx = ref()


def merge(**fields: Any) -> Merge:
    """Output transform merging computed keys into the context:

        output_as=jackdaw.merge(scoped_pbf_file=ctx.working_dir / "out.pbf")
        # -> '$input + {scoped_pbf_file: ($input.working_dir + "/out.pbf")}'
    """
    if not fields:
        raise ValueError("merge() needs at least one key")
    return Merge({k: _lift(v) for k, v in fields.items()})


def join(sep: str, *parts: Any) -> Expr:
    """Join values into one string with a separator, coercing non-strings:

        jackdaw.join(",", ctx.bbox.xmin, ctx.bbox.ymin)
        # -> '${ (.bbox.xmin | tostring) + "," + (.bbox.ymin | tostring) }'
    """
    if not parts:
        raise ValueError("join() needs at least one part")
    result: Expr | None = None
    for part in parts:
        piece = _tostring(_lift(part))
        if result is None:
            result = piece
        else:
            result = _BinOp("+", _BinOp("+", result, _Lit(sep)), piece)
    assert result is not None
    return result


# --------------------------------------------------------------------- #
# context-aware conversion (used by task factories; not user-facing)
# --------------------------------------------------------------------- #


def as_arg(value: Any) -> Any:
    """Task-argument position: Exprs get the ${ ... } wrapper."""
    return f"${{ {value._compile('.')} }}" if isinstance(value, Expr) else value


def as_when(value: "str | Expr") -> str:
    """`when:` position: bare JQ, context root is `.`."""
    return value._compile(".") if isinstance(value, Expr) else value


def as_output_as(value: Any) -> Any:
    """`output.as` position: bare JQ, context root is `$input`
    (`.` is the task's raw output there)."""
    return value._compile("$input") if isinstance(value, Expr) else value


def as_in(value: "str | Expr") -> str:
    """`for.in:` position: bare JQ, context root is `.`."""
    return value._compile(".") if isinstance(value, Expr) else value
