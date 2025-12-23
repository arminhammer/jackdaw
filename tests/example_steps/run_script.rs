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

    // Handle both string output (scripts) and structured output with stdout field (containers)
    let stdout = if let Some(stdout_field) = output.get("stdout") {
        // Structured output like {"stdout": "...", "stderr": "...", "exit_code": 0}
        stdout_field
            .as_str()
            .expect("stdout field is not a string")
    } else if let Some(output_str) = output.as_str() {
        // Direct string output from scripts
        output_str
    } else {
        panic!("Output is neither a string nor an object with stdout field: {:?}", output);
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
