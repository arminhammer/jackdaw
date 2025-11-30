use crate::CtKWorld;
use crate::common::WorkflowStatus;
use cucumber::then;

// Try-specific step: check that workflow output has specific properties
#[then(regex = r"^the workflow output should have properties (.+)$")]
async fn then_output_has_properties(world: &mut CtKWorld, properties: String) {
    let output = world
        .workflow_output
        .as_ref()
        .expect("No workflow output found");

    // Parse the comma-separated list of properties (including nested ones with dot notation)
    // Remove quotes and whitespace
    let property_list: Vec<&str> = properties
        .split(',')
        .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
        .collect();

    for property_path in property_list {
        // Navigate to the property (handle nested paths like 'error.type')
        let parts: Vec<&str> = property_path.split('.').collect();
        let mut current = output;

        for part in &parts {
            match current.get(part) {
                Some(value) => current = value,
                None => panic!(
                    "Property '{}' not found in output. Path: '{}', Output: {:?}",
                    part, property_path, output
                ),
            }
        }
    }
}

// Try-specific step: check that a property has a specific value
#[then(expr = "the workflow output should have a {string} property with value:")]
async fn then_output_has_property_with_value(
    world: &mut CtKWorld,
    property_path: String,
    step: &cucumber::gherkin::Step,
) {
    let output = world
        .workflow_output
        .as_ref()
        .expect("No workflow output found");

    // Navigate to the property
    let parts: Vec<&str> = property_path.split('.').collect();
    let mut current = output;

    for part in &parts {
        current = current.get(part).expect(&format!(
            "Property '{}' not found in path '{}'",
            part, property_path
        ));
    }

    // Parse expected value from docstring, removing YAML marker if present
    use crate::common::parse_docstring;
    let parsed = parse_docstring(step.docstring.as_ref().unwrap());
    let expected_str = parsed.trim();

    // Try to parse as YAML first (to handle both strings and objects)
    if let Ok(expected_value) = serde_yaml::from_str::<serde_json::Value>(expected_str) {
        // Compare JSON values
        assert_eq!(
            current, &expected_value,
            "Property '{}' value mismatch. Expected: {:?}, Actual: {:?}",
            property_path, expected_value, current
        );
    } else {
        // Fall back to string comparison
        let actual_str = if let Some(s) = current.as_str() {
            s
        } else {
            panic!(
                "Property '{}' is not a string and couldn't parse expected as YAML: {:?}",
                property_path, current
            );
        };

        assert_eq!(
            actual_str, expected_str,
            "Property '{}' value mismatch. Expected: '{}', Actual: '{}'",
            property_path, expected_str, actual_str
        );
    }
}

// Try-specific step: check that the workflow faulted (without checking specific error)
#[then(expr = "the workflow should fault")]
async fn then_workflow_faults(world: &mut CtKWorld) {
    match &world.workflow_status {
        Some(WorkflowStatus::Faulted(_)) => {
            // Success - workflow faulted as expected
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
