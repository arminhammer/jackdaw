use crate::ExampleWorld;
use crate::common::WorkflowStatus;
use cucumber::then;
use serde_json::Value;

#[then(expr = "the workflow output should contain stdout with {string}")]
async fn then_output_contains_stdout(world: &mut ExampleWorld, expected_text: String) {
    assert_eq!(
        world.workflow_status,
        Some(WorkflowStatus::Completed),
        "Expected workflow to complete"
    );

    let output = world
        .workflow_output
        .as_ref()
        .expect("No workflow output found");

    // The output should be a JSON object with stdout field
    let stdout = output
        .get("stdout")
        .expect("No stdout field in output")
        .as_str()
        .expect("stdout is not a string");

    assert!(
        stdout.contains(&expected_text),
        "Expected stdout to contain '{}', but got: {}",
        expected_text,
        stdout
    );
}

#[then(expr = "the workflow output should match:")]
async fn then_output_matches(world: &mut ExampleWorld, step: &cucumber::gherkin::Step) {
    use crate::common::parse_docstring;

    assert_eq!(
        world.workflow_status,
        Some(WorkflowStatus::Completed),
        "Expected workflow to complete"
    );

    let expected: Value =
        serde_yaml::from_str(&parse_docstring(step.docstring.as_ref().unwrap())).unwrap();
    assert_eq!(world.workflow_output.as_ref().unwrap(), &expected);
}
