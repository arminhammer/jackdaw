#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use jackdaw::DurableEngineBuilder;
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
use std::time::Duration;

/// Helper to set up test infrastructure
async fn setup_test_engine() -> (DurableEngine, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let persistence = Arc::new(RedbPersistence::new(db_path.to_str().unwrap()).unwrap());
    let cache =
        Arc::new(RedbCache::new(Arc::clone(&persistence.db)).unwrap()) as Arc<dyn CacheProvider>;
    let engine = DurableEngineBuilder::new()
        .with_persistence(Arc::clone(&persistence) as Arc<dyn PersistenceProvider>)
        .with_cache(Arc::clone(&cache))
        .build()
        .unwrap();
    (engine, temp_dir)
}

#[tokio::test]
async fn test_container_environment_variables() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/containers/container-env-vars.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture).unwrap();
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let handle = engine.execute(workflow, json!({})).await.unwrap();
    let result = handle.wait_for_completion(Duration::from_secs(60)).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    let output = result.unwrap();

    // The output is now the direct string result, not an object with stdout/stderr/exitCode
    let output_str = output.as_str().expect("Output should be a string");
    assert!(
        output_str.contains("MY_VAR=HelloWorld"),
        "Output should contain MY_VAR value: {}",
        output_str
    );
    assert!(
        output_str.contains("ANOTHER=TestValue"),
        "Output should contain ANOTHER value: {}",
        output_str
    );
}

#[tokio::test]
async fn test_container_volume_mapping() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create the mount directory that the workflow expects
    let mount_dir = std::path::Path::new("/tmp/container-test");
    std::fs::create_dir_all(mount_dir).expect("Failed to create mount directory");

    let fixture = PathBuf::from("tests/fixtures/containers/container-volume-write.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture).unwrap();
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let handle = engine.execute(workflow, json!({})).await.unwrap();
    let result = handle.wait_for_completion(Duration::from_secs(60)).await;
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
    let workflow_yaml = std::fs::read_to_string(&fixture).unwrap();
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let handle = engine.execute(workflow, json!({})).await.unwrap();
    let result = handle.wait_for_completion(Duration::from_secs(60)).await;
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
    let workflow_yaml = std::fs::read_to_string(&fixture).unwrap();
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let handle = engine.execute(workflow, json!({})).await.unwrap();
    let result = handle.wait_for_completion(Duration::from_secs(60)).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    let output = result.unwrap();

    // The output is now the direct string result, not an object with stdout/stderr/exitCode
    let output_str = output.as_str().expect("Output should be a string");
    assert!(
        output_str.contains("USER=Alice"),
        "Should contain evaluated user: {}",
        output_str
    );
    assert!(
        output_str.contains("COUNT=42"),
        "Should contain evaluated count: {}",
        output_str
    );
}

#[tokio::test]
async fn test_container_stdin_and_arguments() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/containers/container-stdin-args.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture).unwrap();
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let handle = engine.execute(workflow, json!({})).await.unwrap();
    let result = handle.wait_for_completion(Duration::from_secs(60)).await;
    assert!(
        result.is_ok(),
        "Workflow should complete successfully: {:?}",
        result.err()
    );

    let output = result.unwrap();

    // The output is now the direct string result, not an object with stdout/stderr/exitCode
    let output_str = output.as_str().expect("Output should be a string");
    assert!(
        output_str.contains("STDIN:test input"),
        "Should contain stdin: {}",
        output_str
    );
    assert!(
        output_str.contains("ARGS:arg1,arg2"),
        "Should contain arguments: {}",
        output_str
    );
}
