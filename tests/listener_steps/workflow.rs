use crate::ListenerWorld;
use crate::common::parse_docstring;
use cucumber::{given, then, when};
use serde_json::Value;

// Workflow definition step (reuse common workflow parsing)
#[given(expr = "the workflow definition:")]
async fn given_workflow_definition(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let workflow_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    world.workflow_definition = Some(workflow_yaml);
}

// Workflow definition step for gRPC Python
#[given(expr = "a gRPC python workflow with definition:")]
async fn given_grpc_python_workflow(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let workflow_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    world.workflow_definition = Some(workflow_yaml);
}

// Workflow definition step for gRPC TypeScript
#[given(expr = "a gRPC typescript workflow with definition:")]
async fn given_grpc_typescript_workflow(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let workflow_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    world.workflow_definition = Some(workflow_yaml);
}

// Workflow definition step for HTTP Python (note typo in feature file: HTTTP)
#[given(expr = "an HTTTP python workflow with definition:")]
async fn given_http_python_workflow(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let workflow_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    world.workflow_definition = Some(workflow_yaml);
}

// Workflow definition step for HTTP TypeScript
#[given(expr = "an HTTP typescript workflow with definition:")]
async fn given_http_typescript_workflow(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let workflow_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    world.workflow_definition = Some(workflow_yaml);
}

// Workflow input step
#[given(expr = "the workflow input is:")]
async fn given_workflow_input(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let input_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    let input: Value = serde_yaml::from_str(&input_yaml).expect("Failed to parse input");
    world.workflow_input = Some(input);
}

// Execute workflow step
#[when(expr = "the workflow is executed")]
async fn when_workflow_executed(_world: &mut ListenerWorld) {
    // TODO: Implement workflow execution with listener support
    println!("TODO: Execute workflow with listeners");
}

// Check workflow output step
#[then(expr = "the workflow output should be:")]
async fn then_workflow_output(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let expected_yaml = parse_docstring(step.docstring.as_ref().unwrap());
    let expected: Value =
        serde_yaml::from_str(&expected_yaml).expect("Failed to parse expected output");

    let actual = world.workflow_output.as_ref().expect("No workflow output");
    assert_eq!(actual, &expected, "Workflow output mismatch");
}
