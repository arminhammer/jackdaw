use crate::CtKWorld;
use crate::common::{WorkflowStatus, parse_docstring};
use cucumber::then;
use serde_json::Value;

// Raise-specific step: check workflow faulted with specific error
#[then(expr = "the workflow should fault with error:")]
async fn then_workflow_faults_with_error(world: &mut CtKWorld, step: &cucumber::gherkin::Step) {
    // Check that workflow faulted
    match &world.workflow_status {
        Some(WorkflowStatus::Faulted(error_msg)) => {
            // Parse expected error from docstring
            let expected: Value =
                serde_yaml::from_str(&parse_docstring(step.docstring.as_ref().unwrap()))
                    .expect("Failed to parse expected error");

            // Parse the actual error message (it should be a JSON string)
            let actual: Value = serde_json::from_str(&error_msg)
                .expect("Failed to parse actual error");

            // Compare the error fields
            if let (Some(expected_obj), Some(actual_obj)) = (expected.as_object(), actual.as_object()) {
                // Check status
                if let Some(expected_status) = expected_obj.get("status") {
                    assert_eq!(
                        actual_obj.get("status"),
                        Some(expected_status),
                        "Error status mismatch"
                    );
                }

                // Check type
                if let Some(expected_type) = expected_obj.get("type") {
                    assert_eq!(
                        actual_obj.get("type"),
                        Some(expected_type),
                        "Error type mismatch"
                    );
                }

                // Check title
                if let Some(expected_title) = expected_obj.get("title") {
                    assert_eq!(
                        actual_obj.get("title"),
                        Some(expected_title),
                        "Error title mismatch"
                    );
                }

                // Check instance (optional)
                if let Some(expected_instance) = expected_obj.get("instance") {
                    assert_eq!(
                        actual_obj.get("instance"),
                        Some(expected_instance),
                        "Error instance mismatch"
                    );
                }
            } else {
                panic!("Expected and actual errors are not valid objects");
            }
        }
        Some(WorkflowStatus::Completed) => {
            panic!("Expected workflow to fault, but it completed successfully");
        }
        Some(WorkflowStatus::Cancelled) => {
            panic!("Expected workflow to fault, but it was cancelled");
        }
        None => {
            panic!("Expected workflow to fault, but no status was set");
        }
    }
}
