import pytest
from serverlessworkflow.sdk import Workflow, Document

import jackdaw


@pytest.fixture
def hello_world_workflow() -> Workflow:
    return Workflow(
        document=Document(
            dsl="1.0.2",
            namespace="tests",
            name="hello-world",
            version="1.0.0",
        ),
        do=[{"say": {"set": {"statement": "Hello, World!"}}}],
        output={"as": ".statement"},
    )


def test_run_returns_result(hello_world_workflow: Workflow) -> None:
    result = jackdaw.run(hello_world_workflow, timeout=10.0)
    assert result == "Hello, World!"


@pytest.mark.asyncio
async def test_run_async_returns_result(hello_world_workflow: Workflow) -> None:
    result = await jackdaw.run_async(hello_world_workflow, timeout=10.0)
    assert result == "Hello, World!"


def test_workflow_runner_reuse(hello_world_workflow: Workflow) -> None:
    runner = jackdaw.WorkflowRunner()
    result1 = runner.run(hello_world_workflow, timeout=10.0)
    result2 = runner.run(hello_world_workflow, timeout=10.0)
    assert result1 == result2 == "Hello, World!"


def test_run_with_input() -> None:
    workflow = Workflow(
        document=Document(
            dsl="1.0.2",
            namespace="tests",
            name="echo-input",
            version="1.0.0",
        ),
        do=[{"echo": {"set": {"result": "${ .value }"}}}],
        output={"as": ".result"},
    )
    result = jackdaw.run(workflow, input={"value": 42}, timeout=10.0)
    assert result == 42
