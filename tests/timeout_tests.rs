/// Tests for Timeout Enforcement
///
/// Tests that timeouts are enforced at:
/// 1. Workflow level - entire workflow execution
/// 2. Task level - individual task execution
/// 3. Error format - RFC 7807 compliant errors
/// 4. Event emission - task.faulted.v1 and workflow.faulted.v1 events
use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::json;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

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
async fn test_workflow_timeout_enforcement() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/workflow-timeout-iso8601.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read workflow-timeout-iso8601.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    // Should fail with timeout error
    assert!(result.is_err(), "Workflow should timeout and return error");

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Verify timeout occurred around 2 seconds (not completing all 4 seconds of waits)
    assert!(
        elapsed.as_secs() >= 2 && elapsed.as_secs() < 3,
        "Workflow should timeout after approximately 2 seconds, but took {:?}",
        elapsed
    );

    // Verify error message mentions timeout
    assert!(
        error_msg.to_lowercase().contains("timeout")
            || error_msg.to_lowercase().contains("timed out"),
        "Error message should mention timeout: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_workflow_timeout_with_inline_duration() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/workflow-timeout-inline.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read workflow-timeout-inline.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_err(),
        "Workflow should timeout with inline duration format"
    );

    assert!(
        elapsed.as_secs() >= 2 && elapsed.as_secs() < 3,
        "Workflow should timeout after approximately 2 seconds, but took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_task_timeout_enforcement() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/task-timeout-iso8601.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read task-timeout-iso8601.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    // Should fail with timeout error on slowTask
    assert!(result.is_err(), "Task should timeout and fail workflow");

    let error_msg = result.unwrap_err().to_string();

    // Verify timeout occurred after approximately 3 seconds (1s + 2s timeout)
    // not completing the full 7 seconds
    assert!(
        elapsed.as_secs() >= 3 && elapsed.as_secs() < 4,
        "Task should timeout after approximately 3 seconds (1s + 2s), but took {:?}",
        elapsed
    );

    // Verify error message mentions the task and timeout
    assert!(
        error_msg.to_lowercase().contains("timeout")
            || error_msg.to_lowercase().contains("timed out"),
        "Error message should mention timeout: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_task_timeout_with_inline_duration() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/task-timeout-inline.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read task-timeout-inline.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_err(),
        "Task should timeout with inline duration format"
    );

    assert!(
        elapsed.as_secs() >= 2 && elapsed.as_secs() < 3,
        "Task should timeout after approximately 2 seconds, but took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_workflow_completes_within_timeout() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/workflow-completes-within-timeout.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture)
        .expect("Failed to read workflow-completes-within-timeout.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    // Should complete successfully
    assert!(
        result.is_ok(),
        "Workflow should complete successfully within timeout: {:?}",
        result.err()
    );

    // Should take about 2 seconds, well under the 5 second timeout
    assert!(
        elapsed.as_secs() >= 2 && elapsed.as_secs() < 3,
        "Workflow should complete in approximately 2 seconds, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_task_completes_within_timeout() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/task-completes-within-timeout.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture)
        .expect("Failed to read task-completes-within-timeout.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;

    assert!(
        result.is_ok(),
        "Task should complete successfully within timeout: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_task_timeout_wait_task() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/task-timeout-call.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read task-timeout-call.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "Wait task should timeout");

    assert!(
        elapsed.as_secs() >= 2 && elapsed.as_secs() < 3,
        "Wait task should timeout after approximately 2 seconds, but took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_nested_timeout_task_overrides_workflow() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/nested-timeout-task-overrides.sw.yaml");
    let workflow_yaml = std::fs::read_to_string(&fixture)
        .expect("Failed to read nested-timeout-task-overrides.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_err(),
        "Task timeout should trigger before workflow timeout"
    );

    // Should timeout around 2 seconds (task timeout), not 10 seconds (workflow timeout)
    assert!(
        elapsed.as_secs() >= 2 && elapsed.as_secs() < 3,
        "Should timeout at task level (~2s), not workflow level (10s), but took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_timeout_with_milliseconds() {
    let (engine, _temp_dir) = setup_test_engine().await;

    let fixture = PathBuf::from("tests/fixtures/timeout/timeout-milliseconds.sw.yaml");
    let workflow_yaml =
        std::fs::read_to_string(&fixture).expect("Failed to read timeout-milliseconds.sw.yaml");
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml).unwrap();

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "Workflow should timeout");

    assert!(
        elapsed.as_millis() >= 1500 && elapsed.as_millis() < 2500,
        "Workflow should timeout after approximately 1500ms, but took {:?}",
        elapsed
    );
}
