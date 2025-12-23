/// Container Task Feature Tests
///
/// Tests for container advanced features per Serverless Workflow spec:
/// - Volume mappings (host paths mounted in container)
/// - Environment variables (key-value pairs passed to container)
/// - Expression evaluation in environment variables
/// - Stdin and arguments support
use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::json;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::path::PathBuf;
use std::sync::Arc;

/// Helper to set up test infrastructure
async fn setup_test_engine() -> (Arc<DurableEngine>, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let persistence = Arc::new(RedbPersistence::new(db_path.to_str().unwrap()).unwrap());
    let cache =
        Arc::new(RedbCache::new(Arc::clone(&persistence.db)).unwrap()) as Arc<dyn CacheProvider>;
    let engine = Arc::new(
        DurableEngine::new(
            Arc::clone(&persistence) as Arc<dyn PersistenceProvider>,
            Arc::clone(&cache),
        )
        .unwrap(),
    );
    (engine, temp_dir)
}

#[tokio::test]
async fn test_container_environment_variables() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/containers/container-env-vars.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read container-env-vars.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    let (_instance_id, output) = result.unwrap();

    // Debug: print the entire output structure
    eprintln!("Full output: {:?}", output);

    // Verify environment variables were used
    let stdout = output.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    eprintln!("stdout value: '{}'", stdout);
    assert!(
        stdout.contains("MY_VAR=HelloWorld"),
        "Output should contain MY_VAR value: {}",
        stdout
    );
    assert!(
        stdout.contains("ANOTHER=TestValue"),
        "Output should contain ANOTHER value: {}",
        stdout
    );
}

#[tokio::test]
async fn test_container_volume_mapping() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create the mount directory that the workflow expects
    let mount_dir = std::path::Path::new("/tmp/container-test");
    std::fs::create_dir_all(mount_dir).expect("Failed to create mount directory");

    let fixture = PathBuf::from("tests/fixtures/containers/container-volume-write.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read container-volume-write.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    // Verify file was created on host
    let test_file = mount_dir.join("test.txt");
    assert!(test_file.exists(), "File should exist on host filesystem");

    let content = std::fs::read_to_string(&test_file).expect("Failed to read output file");
    assert_eq!(content.trim(), "Hello from container");

    // Cleanup
    let _ = std::fs::remove_file(&test_file);
    let _ = std::fs::remove_dir(mount_dir);
}

#[tokio::test]
async fn test_container_multiple_volumes() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create input and output directories
    let input_dir = std::path::Path::new("/tmp/container-test-input");
    let output_dir = std::path::Path::new("/tmp/container-test-output");
    std::fs::create_dir_all(input_dir).expect("Failed to create input directory");
    std::fs::create_dir_all(output_dir).expect("Failed to create output directory");

    // Create input file
    let input_file = input_dir.join("input.txt");
    std::fs::write(&input_file, "Input data").expect("Failed to write input file");

    let fixture = PathBuf::from("tests/fixtures/containers/container-multiple-volumes.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture)
        .expect("Failed to read container-multiple-volumes.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    // Verify output file was created
    let output_file = output_dir.join("output.txt");
    assert!(output_file.exists(), "Output file should exist");

    let content = std::fs::read_to_string(&output_file).expect("Failed to read output file");
    assert_eq!(content.trim(), "INPUT DATA");

    // Cleanup
    let _ = std::fs::remove_file(&input_file);
    let _ = std::fs::remove_file(&output_file);
    let _ = std::fs::remove_dir(input_dir);
    let _ = std::fs::remove_dir(output_dir);
}

#[tokio::test]
async fn test_container_environment_with_expressions() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/containers/container-env-expressions.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture)
        .expect("Failed to read container-env-expressions.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    let (_instance_id, output) = result.unwrap();

    // Verify expressions were evaluated
    let stdout = output.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        stdout.contains("USER=Alice"),
        "Should contain evaluated user: {}",
        stdout
    );
    assert!(
        stdout.contains("COUNT=42"),
        "Should contain evaluated count: {}",
        stdout
    );
}

#[tokio::test]
async fn test_container_stdin_and_arguments() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/containers/container-stdin-args.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read container-stdin-args.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    let (_instance_id, output) = result.unwrap();

    let stdout = output.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        stdout.contains("STDIN:test input"),
        "Should contain stdin: {}",
        stdout
    );
    assert!(
        stdout.contains("ARGS:arg1,arg2"),
        "Should contain arguments: {}",
        stdout
    );
}
