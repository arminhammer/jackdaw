#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::collapsible_if)]

use jackdaw::durableengine::DurableEngine;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::path::PathBuf;

/// Helper function to validate a single workflow file
fn validate_workflow_file(path: &PathBuf) -> Result<(), String> {
    // Read the workflow file
    let workflow_yaml = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // Parse the workflow
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    // Validate graph structure
    DurableEngine::validate_workflow_graph(&workflow)
        .map_err(|e| format!("Graph validation failed for {}: {}", path.display(), e))?;

    Ok(())
}

/// Test that validates all example workflow files in submodules/specification/examples
///
/// This is a special test that ensures all official examples can at least
/// be validated (dry run). We may not be able to realistically execute all
/// of them, but we can test our validation logic against them.
#[tokio::test]
async fn test_validate_all_examples() {
    // Files to skip - examples that have incorrect syntax not matching the spec
    let skip_files: Vec<&str> = vec![];

    // Path to the examples directory
    let examples_dir = PathBuf::from("submodules/specification/examples");

    // Verify the directory exists
    assert!(
        examples_dir.exists(),
        "Examples directory does not exist: {}",
        examples_dir.display()
    );

    // Collect all .yaml and .yml files
    let mut workflow_files = Vec::new();
    for entry in std::fs::read_dir(&examples_dir).expect("Failed to read examples directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "yaml" || ext == "yml" {
                    // Skip files that are known to have incorrect syntax
                    let file_name = path.file_name().unwrap().to_string_lossy();
                    if !skip_files.iter().any(|f| *f == file_name.as_ref()) {
                        workflow_files.push(path);
                    }
                }
            }
        }
    }

    // Sort for consistent test output
    workflow_files.sort();

    assert!(
        !workflow_files.is_empty(),
        "No workflow files found in {}",
        examples_dir.display()
    );

    println!(
        "Found {} workflow files to validate ({} skipped)",
        workflow_files.len(),
        skip_files.len()
    );

    // Validate each workflow
    let mut failed_workflows = Vec::new();
    for workflow_path in &workflow_files {
        match validate_workflow_file(workflow_path) {
            Ok(_) => {
                println!("✓ {}", workflow_path.display());
            }
            Err(e) => {
                println!("✗ {}", e);
                failed_workflows.push((workflow_path.clone(), e));
            }
        }
    }

    // Report results
    if !failed_workflows.is_empty() {
        let mut error_msg = format!(
            "\n{} out of {} workflows failed validation:\n",
            failed_workflows.len(),
            workflow_files.len()
        );
        for (path, err) in failed_workflows {
            error_msg.push_str(&format!("  - {}: {}\n", path.display(), err));
        }
        panic!("{}", error_msg);
    }

    println!(
        "\n✓ All {} examples validated successfully!",
        workflow_files.len()
    );
}

/// Test individual example files
///
/// This test validates a specific example file to ensure it passes validation.
#[tokio::test]
async fn test_validate_for_example() {
    let example_file = PathBuf::from("submodules/specification/examples/for.yaml");

    assert!(
        example_file.exists(),
        "Example file does not exist: {}",
        example_file.display()
    );

    let result = validate_workflow_file(&example_file);

    assert!(
        result.is_ok(),
        "Validation failed for for.yaml: {:?}",
        result.err()
    );
}

/// Test that validation properly fails for invalid workflow files
#[tokio::test]
async fn test_validate_nonexistent_file() {
    let nonexistent_file =
        PathBuf::from("submodules/specification/examples/this-does-not-exist.yaml");

    let result = validate_workflow_file(&nonexistent_file);

    // Should fail because file doesn't exist
    assert!(
        result.is_err(),
        "Expected validation to fail for nonexistent file"
    );
}
