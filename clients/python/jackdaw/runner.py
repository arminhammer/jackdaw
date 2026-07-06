"""Workflow composition and one-shot execution.

`WorkflowBuilder` assembles workflows from Python functions, shell commands,
and containers; `run` / `run_async` execute an assembled workflow through the
jackdaw engine. For interactive, step-at-a-time construction in notebooks use
:class:`jackdaw.Session` instead — builders remain the composition layer for
control-flow blocks (`for_loop`, `fork`) committed into a session as a single
step.
"""

import asyncio
import textwrap
from collections.abc import Callable
from typing import Any, Protocol, runtime_checkable

from serverlessworkflow.sdk import Workflow
from serverlessworkflow.sdk.base import Output, TaskItem
from serverlessworkflow.sdk.tasks import (
    ContainerConfiguration,
    DoTask,
    ForConfiguration,
    ForkConfiguration,
    ForkTask,
    ForTask,
    RunConfiguration,
    RunTask,
    ScriptConfiguration,
    ShellConfiguration,
)
from dataclasses import dataclass

from serverlessworkflow.sdk.workflow import Document

# Re-exported for backwards compatibility (tests and downstream imports).
from ._expr import Expr, as_arg, as_in, as_output_as
from ._codegen import (  # noqa: F401
    _MAIN_BLOCK,
    _MERGE_OUTPUT,
    _PASSTHROUGH_OUTPUT,
    _extract_helper_functions,
    _extract_module_imports,
    _imports_from_globals,
    _imports_from_source,
    _input_schema,
    function_to_task,
)
from ._jackdaw import DurableEngineBuilder, ExecutionHandle

# Typed task markers: each specializes RunTask to one process kind so steps
# are well-typed objects (a session step is either a typed Python function or
# one of these). They add no fields — `run` stays a RunConfiguration with the
# matching process set, which is what serializes to valid workflow YAML
# (run: {shell: ...}, run: {script: ...}, run: {container: ...}).


@dataclass
class RunShellTask(RunTask):
    """A run task executing a shell command (``run.shell``)."""


@dataclass
class RunScriptTask(RunTask):
    """A run task executing a script (``run.script``).

    Typed Python step functions compile to script tasks automatically — this
    type is for predefined script helpers like ``build_image_task`` and
    ``stop_container_task`` that generate their code.
    """


@dataclass
class RunContainerTask(RunTask):
    """A run task executing a container (``run.container``)."""


def container_task(
    image: str,
    command: str | None = None,
    arguments: "list[str | Expr] | None" = None,
    environment: "dict[str, str | Expr] | None" = None,
    volumes: "dict[str | Expr, str | Expr] | None" = None,
    ports: dict[int, int] | None = None,
    name: str | None = None,
    output_as: "str | Expr | None" = None,
) -> RunContainerTask:
    """Standalone factory for a reusable container step.

    Volume keys and values may be JQ expressions evaluated against the context at runtime:

        valhalla_container = container_task(
            image="ghcr.io/valhalla/valhalla:run-latest",
            command="valhalla_build_tiles -c /data/valhalla.json /data/scoped.osm.pbf",
            volumes={"${ .working_dir }": "/data"},
        )

    Use `name` to assign a stable container name — required for the fork-as-service
    pattern where a sibling branch stops the container by name.
    Use `ports` to publish ports: {container_port: host_port}.
    """
    return RunContainerTask(
        run=RunConfiguration(
            container=ContainerConfiguration(
                image=image,
                name=name,
                command=command,
                arguments=[as_arg(a) for a in arguments] if arguments else arguments,
                environment=(
                    {k: as_arg(v) for k, v in environment.items()} if environment else environment
                ),
                volumes=(
                    {as_arg(k): as_arg(v) for k, v in volumes.items()} if volumes else volumes
                ),
                ports=ports,
            ),
        ),
        output=Output(as_=as_output_as(output_as) or _PASSTHROUGH_OUTPUT),
    )


def build_image_task(
    tag: str,
    dockerfile: str,
    context_dir: str = "",
) -> RunScriptTask:
    """Generate a Python script step that builds a Docker image via jackdaw's bollard runtime.

    tag          — image tag to apply, e.g. "sedona-spark4:local"
    dockerfile   — Dockerfile content as a string
    context_dir  — path to the build context directory; pass "" to use a temp dir
                   containing only the Dockerfile

    Respects .dockerignore in context_dir. Build output streams to stderr in real time.
    The step passes the workflow context through unchanged.
    """
    code = (
        "import sys\n"
        "import jackdaw._jackdaw as _jd\n"
        "\n"
        "if __name__ == '__main__':\n"
        f"    print('Building image {tag}...', flush=True)\n"
        f"    _jd.build_image({tag!r}, {dockerfile!r}, {context_dir!r})\n"
        f"    print('Image {tag} built successfully.', flush=True)\n"
    )

    return RunScriptTask(
        run=RunConfiguration(
            script=ScriptConfiguration(
                language="python",
                code=code,
                stdin="${ . }",
            ),
        ),
        output=Output(as_=_PASSTHROUGH_OUTPUT),
    )


def stop_container_task(container_name: str) -> RunScriptTask:
    """Generate a Python script step that stops a named container via jackdaw's bollard runtime.

    container_name — literal container name, e.g. "sedona".

    The generated step calls `jackdaw._jackdaw.stop_container(name)` in a subprocess using the
    same interpreter (and virtualenv) that launched the pipeline — no docker CLI required.
    """
    code = textwrap.dedent(f"""\
        import sys
        import json
        import jackdaw._jackdaw as _jd

        if __name__ == "__main__":
            _jd.stop_container({container_name!r})
    """)

    return RunScriptTask(
        run=RunConfiguration(
            script=ScriptConfiguration(
                language="python",
                code=code,
                stdin="${ . }",
            ),
        ),
        output=Output(as_=_PASSTHROUGH_OUTPUT),
    )

def shell_task(
    command: str,
    arguments: "list[str | Expr] | None" = None,
    environment: "dict[str, str | Expr] | None" = None,
    output_as: "str | Expr | None" = None,
) -> RunShellTask:
    """Standalone factory for a reusable shell step.

    Arguments bind to context keys at runtime — pass `jackdaw.ref(...)` /
    `jackdaw.ctx` expressions (or raw JQ strings) at call-site:

        ctx = jackdaw.ref(OsmInput)
        shell_task("osmium", [
            "extract", "-p", ctx.combined_geojson,
            ctx.raw_pbf_file, "-o", ctx.working_dir / "valhalla_runs/scoped.osm.pbf",
        ], output_as=jackdaw.merge(scoped_pbf_file=ctx.working_dir / "valhalla_runs/scoped.osm.pbf"))
    """
    return RunShellTask(
        run=RunConfiguration(
            shell=ShellConfiguration(
                command=command,
                arguments=[as_arg(a) for a in arguments] if arguments else arguments,
                environment=(
                    {k: as_arg(v) for k, v in environment.items()} if environment else environment
                ),
            ),
        ),
        output=Output(as_=as_output_as(output_as) or _PASSTHROUGH_OUTPUT),
    )

class WorkflowBuilder:
    """Composes a Workflow from Python functions, shell commands, and containers.

    In notebooks, prefer :class:`jackdaw.Session` for step-at-a-time
    construction; builders are the composition layer for multi-task blocks
    (fork branches, for-loop bodies) committed into a session as one step.
    """

    def __init__(self, name: str = "", namespace: str = "default", version: str = "0.1.0") -> None:
        self._name = name
        self._namespace = namespace
        self._version = version
        self._steps: list[TaskItem] = []

    def add(self, name: str, fn_or_task: "Callable | RunTask") -> "WorkflowBuilder":
        """Add a step by name.

        Accepts either a plain Python callable or a pre-built ``RunTask``
        (from ``shell_task``, ``container_task``, ``mkdir``, etc.):

            pipeline.add("filter-feeds", filter_feeds)          # callable
            pipeline.add("make-dir",     mkdir("${ .working_dir }"))  # RunTask

        For Python callables, module-level imports are detected and prepended
        automatically.  The function receives its declared parameters by name
        from the accumulated context (not positional argv), so non-string
        types are preserved.  Its return dict is merged into the context for
        subsequent steps.
        """
        if callable(fn_or_task):
            self._steps.append(TaskItem(name=name, task=function_to_task(fn_or_task)))
        else:
            self._steps.append(TaskItem(name=name, task=fn_or_task))
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
        task = shell_task(
            command=command,
            arguments=arguments,
            environment=environment,
            output_as=output_as,
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
        ports: dict[int, int] | None = None,
        container_name: str | None = None,
        output_as: str | None = None,
    ) -> "WorkflowBuilder":
        """Add a container step.

        command        — shell script run inside the container via sh -c (supports redirects).
        arguments      — positional $1/$2/… args passed after the command.
        volumes        — host→container path mappings; keys/values may be JQ expressions,
                         e.g. volumes={"${ .working_dir }": "/data"}.
        ports          — port mappings: {container_port: host_port}.
        container_name — stable Docker container name; required when a sibling fork branch
                         needs to stop this container by name.
        output_as      — JQ expression for the output transform; defaults to passthrough.
        """
        task = container_task(
            image=image,
            command=command,
            arguments=arguments,
            environment=environment,
            volumes=volumes,
            ports=ports,
            name=container_name,
            output_as=output_as,
        )
        self._steps.append(TaskItem(name=name, task=task))
        return self

    def for_loop(
        self,
        name: str,
        each: str,
        in_: "str | Expr",
        body: "WorkflowBuilder",
    ) -> "WorkflowBuilder":
        """Add a step that iterates over a collection, executing body steps per item.

        each — variable name injected into context for each item (e.g. "scenario")
        in_  — raw JQ expression evaluating to the array (e.g. ".scenarios")
               Do NOT use the ${ } wrapper here — in_ is a runtime expression
               field that is passed directly to the JQ evaluator.
        body — WorkflowBuilder whose steps run on each iteration

        Each iteration's output is merged into context so subsequent steps and
        later iterations accumulate results. The loop variable is removed from
        context after each iteration.
        """
        self._steps.append(TaskItem(
            name=name,
            task=ForTask(
                for_=ForConfiguration(each=each, in_=as_in(in_)),
                do=body.steps(),
            ),
        ))
        return self

    def fork(self, name: str, branches: dict[str, "WorkflowBuilder"]) -> "WorkflowBuilder":
        """Add parallel branches. Each value is a WorkflowBuilder defining that branch."""
        branch_items = [
            TaskItem(name=branch_name, task=DoTask(do=sub.steps()))
            for branch_name, sub in branches.items()
        ]
        self._steps.append(
            TaskItem(name=name, task=ForkTask(fork=ForkConfiguration(branches=branch_items)))
        )
        return self

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


@runtime_checkable
class Config(Protocol):
    """Protocol for pipeline configuration objects.

    Any class with a `to_pipeline_input` method satisfies this protocol —
    no inheritance required. The method must return a flat dict that jackdaw
    passes as the workflow's initial context.
    """

    def to_pipeline_input(self) -> dict: ...


def run_from_config(
    build_fn: Callable[..., "WorkflowBuilder"],
    cfg: Config,
    timeout: float = 86400.0,
) -> object:
    """Build and run a pipeline from a config object.

    build_fn receives cfg and returns a WorkflowBuilder. cfg.to_pipeline_input()
    supplies the initial workflow context. Runs synchronously.
    """
    pipeline = build_fn(cfg).build()
    return run(pipeline, input=cfg.to_pipeline_input(), timeout=timeout)


async def run_from_config_async(
    build_fn: Callable[..., "WorkflowBuilder"],
    cfg: Config,
    timeout: float = 86400.0,
) -> object:
    """Async variant of run_from_config."""
    pipeline = build_fn(cfg).build()
    return await run_async(pipeline, input=cfg.to_pipeline_input(), timeout=timeout)


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
