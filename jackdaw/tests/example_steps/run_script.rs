#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

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

    // The output can be either:
    // 1. A string (new format - direct stdout)
    // 2. An object with stdout field (old format)
    let stdout = if let Some(stdout_str) = output.as_str() {
        stdout_str
    } else if let Some(stdout_field) = output.get("stdout") {
        stdout_field.as_str().expect("stdout is not a string")
    } else {
        panic!(
            "Output is neither a string nor an object with stdout field: {:?}",
            output
        );
    };

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
