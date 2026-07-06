"""Interactive, context-first workflow construction for notebooks.

A :class:`Session` holds the evolving workflow context and the steps you have
accepted so far. The two core verbs mirror a git-style staging flow:

- :meth:`Session.preview` — run a step against the current context and return
  the candidate result. Nothing is recorded; re-run the cell as often as you
  like while iterating on the function.
- :meth:`Session.commit` — accept the step: execute it, record it, advance
  the context. Committing a name that already exists *replaces* that step
  (the context is rewound to the step's original input first) and marks every
  later step stale; :meth:`Session.replay` re-runs the stale suffix.

When the pipeline looks right, :meth:`Session.export` writes the assembled
workflow YAML — the reproducible artifact. Run it later with
``jackdaw.run(...)`` or the jackdaw CLI; the notebook itself is not the
artifact.

Everything is typed: the workflow's `input` is a typed instance (a pydantic
model or dataclass instance) carrying both the seed values and the input
schema, and its `output` is a type describing the keys the workflow
guarantees. Steps are typed Python functions (parameters name the context
keys they consume; the returned dict merges into the context) — or, for work
that isn't Python, specific task types: `RunShellTask`, `RunContainerTask`,
`RunScriptTask`.

Typical notebook flow::

    import jackdaw

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

    def filter_feeds(gtfs_csv_file: str, bbox: dict) -> dict:
        ...

    out = sess.preview(filter_feeds)     # iterate freely
    out["download_urls"][:5]             # inspect

    sess.commit(filter_feeds)            # accept and advance
    sess.status()                        # what's committed, what's stale
    sess.export("gtfs.sw.yaml")          # the artifact

Note that rewinds restore the *context* only — files or containers created by
previously executed steps are not undone.
"""

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from serverlessworkflow.sdk import Workflow
from serverlessworkflow.sdk.base import TaskItem
from serverlessworkflow.sdk.tasks import DoTask, RunTask, SetTask, SwitchCase, SwitchTask
from serverlessworkflow.sdk.workflow import Document

from ._codegen import function_to_task, required_params
from ._contract import input_contract, output_contract
from ._expr import Expr, as_when
from ._jackdaw import Session as _NativeSession
from .runner import RunContainerTask, RunScriptTask, RunShellTask


@dataclass
class Case:
    """One branch of a :func:`switch` step.

    `when` is a JQ expression evaluated against the context at runtime; the
    first matching case runs. A case without `when` is the default branch.
    """

    name: str
    step: "Step"
    when: "str | Expr | None" = None


def case(name: str, step: "Step", when: "str | Expr | None" = None) -> Case:
    """Declare a switch branch: `case("use-polygon", step, when=(ctx.poly != None))`."""
    return Case(name=name, step=step, when=when)


@dataclass
class Switch:
    """A conditional step: routes to the first matching case at runtime.

    Authoring sugar only — compiles to plain spec constructs (a `switch`
    task, the branch tasks, and a `join` inside one `do` block), so the
    branch decision lives in the exported artifact, driven by the input.
    The Rust engine evaluates and routes. Build with :func:`switch`.
    """

    cases: tuple[Case, ...]


def switch(*cases: Case) -> Switch:
    """Build a conditional step from :func:`case` branches.

        extract_region = jackdaw.switch(
            jackdaw.case("use-polygon", polygon_task, when=".combined_geojson != null"),
            jackdaw.case("use-bbox", bbox_task),  # no `when` → default
        )
        sess.commit(extract_region, name="extract-region")
    """
    if not cases:
        raise ValueError("switch() needs at least one case")
    return Switch(cases=cases)


# A session step: a typed Python function, a specific task type for
# non-Python work (shell commands, containers, scripts), or a runtime
# conditional built with switch().
Step = Callable | RunShellTask | RunContainerTask | RunScriptTask | Switch


class Session:
    """Interactive pipeline session backed by the jackdaw engine.

    A session declares its contract with types: `input` is a typed instance
    (a pydantic model or dataclass instance) carrying both the seed values
    and the input schema; `output` is a type (a TypedDict, pydantic model,
    or dataclass) describing the keys the workflow guarantees.

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

    The seed values are validated against the derived input schema
    immediately, and both schemas are embedded in the exported workflow,
    where the engine enforces them on every execution.
    """

    def __init__(
        self,
        name: str,
        input: Any,
        output: type,
        namespace: str = "default",
        version: str = "0.1.0",
    ) -> None:
        values, input_schema = input_contract(input)
        output_schema = output_contract(output)
        self._name = name
        self._namespace = namespace
        self._version = version
        self._inner = _NativeSession(
            input=values,
            input_schema=input_schema,
            output_schema=output_schema,
        )

    # ------------------------------------------------------------------ #
    # context
    # ------------------------------------------------------------------ #

    @property
    def ctx(self) -> dict:
        """The current accumulated context.

        Returns a copy — mutating it in place has no effect on the session.
        Use :meth:`update` (or assign a whole dict to ``ctx``) to change the
        context between steps.
        """
        return self._inner.ctx

    @ctx.setter
    def ctx(self, value: dict) -> None:
        self._inner.ctx = dict(value)

    def update(self, **kwargs: Any) -> dict:
        """Merge keyword arguments into the context and return it.

        Useful for narrowing data between steps::

            sess.update(download_urls=sess.ctx["download_urls"][:3])
        """
        return self._inner.update(kwargs)

    # ------------------------------------------------------------------ #
    # step execution
    # ------------------------------------------------------------------ #

    def preview(
        self,
        step: "Step",
        name: str | None = None,
        timeout: float = 3600.0,
    ) -> dict:
        """Execute `step` against the current context and return the result.

        The session is not modified: no step is recorded and the context does
        not advance. Preview is idempotent — call it repeatedly while
        iterating on the step definition, then :meth:`commit` when satisfied.

        `step` is a typed Python function (parameters name the context keys
        it consumes; its returned dict merges into the context) or a specific
        task type — `RunShellTask` / `RunContainerTask` / `RunScriptTask` —
        which requires an explicit `name`.
        """
        yaml_doc = self._prepare(step, name)
        return self._inner.preview(yaml_doc, timeout)

    def commit(
        self,
        step: "Step",
        name: str | None = None,
        timeout: float = 3600.0,
    ) -> dict:
        """Accept `step`: execute it, record it, and advance the context.

        Steps are keyed by name (the function name, kebab-cased, unless
        `name` overrides it). Re-committing an existing name rewinds the
        context to that step's original input, executes the new definition in
        its place, and marks every later step stale — so re-running an edited
        commit cell *is* the edit operation. Heal stale steps with
        :meth:`replay`.

        Returns the new context.
        """
        yaml_doc = self._prepare(step, name)
        return self._inner.commit(yaml_doc, timeout)

    def replay(self, timeout: float = 3600.0) -> dict:
        """Re-execute all steps from the first stale one onward, in order.

        Returns the final context. No-op when nothing is stale. `timeout`
        applies per step.
        """
        return self._inner.replay(timeout)

    def rollback(self, name: str) -> dict:
        """Rewind the context to just before `name` ran and drop that step
        and everything after it. Returns the restored context."""
        return self._inner.rollback(name)

    def status(self) -> list[dict]:
        """Committed steps in order as ``{"name": str, "stale": bool}`` dicts."""
        return self._inner.status()

    # ------------------------------------------------------------------ #
    # export
    # ------------------------------------------------------------------ #

    def export(
        self,
        path: str | None = None,
        name: str | None = None,
        namespace: str | None = None,
        version: str | None = None,
    ) -> str:
        """Serialise all committed steps as workflow YAML.

        Writes the document to `path` when given; always returns the YAML
        string. The result is the reproducible pipeline artifact — execute it
        with ``jackdaw.run(...)`` or the jackdaw CLI.
        """
        stale = [s["name"] for s in self.status() if s["stale"]]
        if stale:
            raise RuntimeError(
                f"Cannot export with stale steps: {', '.join(stale)}. "
                "Run sess.replay() first so the export reflects executed state."
            )

        yaml_doc = self._inner.export_yaml(
            name or self._name or "session-pipeline",
            namespace or self._namespace,
            version or self._version,
        )
        if path is not None:
            with open(path, "w", encoding="utf-8") as f:
                f.write(yaml_doc)
        return yaml_doc

    # ------------------------------------------------------------------ #
    # internals
    # ------------------------------------------------------------------ #

    def _prepare(self, step: "Step", name: str | None) -> str:
        """Build the single-step workflow YAML for a step.

        A step is a typed Python function (params = consumed context keys,
        returned dict merges into the context), a specific task type —
        `RunShellTask`, `RunContainerTask`, `RunScriptTask` — for work that
        isn't Python, or a :func:`switch` conditional.
        """
        if isinstance(step, (Switch, RunShellTask, RunScriptTask, RunContainerTask)):
            if name is None:
                raise ValueError(
                    "Task objects need an explicit name: "
                    "commit(mkdir('${ .working_dir }'), name='make-dir')"
                )
            step_name = name
            task = self._switch_to_task(step) if isinstance(step, Switch) else step
        elif callable(step) and not isinstance(step, RunTask):
            step_name = name or step.__name__.replace("_", "-")
            self._check_params(step)
            task = function_to_task(step)
        else:
            raise TypeError(
                "Session steps are typed Python functions or specific task "
                "types (RunShellTask / RunContainerTask / RunScriptTask); "
                f"got: {step!r}"
            )

        workflow = Workflow(
            document=Document(
                dsl="1.0.2",
                namespace=self._namespace,
                name=step_name,
                version=self._version,
            ),
            do=[TaskItem(name=step_name, task=task)],
        )
        return workflow.to_yaml()

    def _switch_to_task(self, sw: Switch) -> DoTask:
        """Compile a Switch into plain spec constructs: one `do` block holding
        the `switch` router, the branch tasks (each flowing to `join`), and a
        pass-through `join`. The branch decision is made by the engine at
        runtime, from the input — not at pipeline-generation time."""
        router: list[dict[str, SwitchCase]] = [
            # as_when compiles a Python Expr (e.g. ctx.poly != None) into the
            # JQ text the spec's YAML carries; raw strings pass through.
            {
                c.name: SwitchCase(
                    when=as_when(c.when) if c.when is not None else None,
                    then=c.name,
                )
            }
            for c in sw.cases
        ]
        if all(c.when is not None for c in sw.cases):
            # No default branch: route unmatched inputs past all branches.
            router.append({"no-match": SwitchCase(then="join")})

        items = [TaskItem(name="choose", task=SwitchTask(switch=router))]
        for c in sw.cases:
            branch_task = self._branch_to_task(c)
            branch_task.then = "join"
            items.append(TaskItem(name=c.name, task=branch_task))
        items.append(TaskItem(name="join", task=SetTask(set="${ . }")))
        return DoTask(do=items)

    def _branch_to_task(self, c: Case):
        """Convert one switch branch's step to a task."""
        if isinstance(c.step, (RunShellTask, RunScriptTask, RunContainerTask)):
            return c.step
        if callable(c.step) and not isinstance(c.step, RunTask):
            self._check_params(c.step)
            return function_to_task(c.step)
        raise TypeError(
            f"switch case {c.name!r}: branch steps are typed Python functions "
            f"or specific task types, got {c.step!r}"
        )

    def _check_params(self, fn: Callable) -> None:
        """Fail fast, in the cell, when required parameters are missing."""
        missing = [p for p in required_params(fn) if p not in self._inner.ctx]
        if missing:
            raise ValueError(
                f"{fn.__name__}() requires context keys that are not present: "
                f"{', '.join(missing)}. Available keys: "
                f"{', '.join(sorted(self._inner.ctx.keys())) or '(none)'}"
            )
