/// Tests for Wait Task implementation
///
/// Tests that wait tasks:
/// 1. Wait for the specified duration (ISO 8601 format)
/// 2. Wait for inline duration format (seconds, minutes, etc.)
/// 3. Measure elapsed time to ensure accuracy
/// 4. Support expression evaluation (if needed)

use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::json;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
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
async fn test_wait_task_iso8601_seconds() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with wait task using ISO 8601 duration (2 seconds)
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-iso8601
  version: '1.0.0'
do:
  - waitTask:
      wait: PT2S
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Wait task should complete successfully");

    // Verify the wait duration (allow some tolerance for execution overhead)
    assert!(
        elapsed.as_secs() >= 2,
        "Wait task should wait at least 2 seconds, but only waited {:?}",
        elapsed
    );
    assert!(
        elapsed.as_secs() < 3,
        "Wait task should not wait much longer than 2 seconds, but waited {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_wait_task_iso8601_minutes() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with wait task using ISO 8601 duration (1 minute = 60 seconds)
    // We'll use PT1M (1 minute) but for testing we'll use a shorter duration
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-iso8601-minutes
  version: '1.0.0'
do:
  - waitTask:
      wait: PT0.05M
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Wait task should complete successfully");

    // 0.05 minutes = 3 seconds
    assert!(
        elapsed.as_secs() >= 3,
        "Wait task should wait at least 3 seconds, but only waited {:?}",
        elapsed
    );
    assert!(
        elapsed.as_secs() < 4,
        "Wait task should not wait much longer than 3 seconds, but waited {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_wait_task_inline_seconds() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with wait task using inline duration format
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-inline-seconds
  version: '1.0.0'
do:
  - waitTask:
      wait:
        seconds: 1
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Wait task should complete successfully");

    // Verify the wait duration
    assert!(
        elapsed.as_secs() >= 1,
        "Wait task should wait at least 1 second, but only waited {:?}",
        elapsed
    );
    assert!(
        elapsed.as_secs() < 2,
        "Wait task should not wait much longer than 1 second, but waited {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_wait_task_inline_milliseconds() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with wait task using inline duration format (milliseconds)
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-inline-milliseconds
  version: '1.0.0'
do:
  - waitTask:
      wait:
        milliseconds: 500
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Wait task should complete successfully");

    // Verify the wait duration
    assert!(
        elapsed.as_millis() >= 500,
        "Wait task should wait at least 500ms, but only waited {:?}",
        elapsed
    );
    assert!(
        elapsed.as_millis() < 1000,
        "Wait task should not wait much longer than 500ms, but waited {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_wait_task_inline_composite() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with wait task using composite duration (1 second + 500 milliseconds)
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-inline-composite
  version: '1.0.0'
do:
  - waitTask:
      wait:
        seconds: 1
        milliseconds: 500
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Wait task should complete successfully");

    // Verify the wait duration (1.5 seconds total)
    assert!(
        elapsed.as_millis() >= 1500,
        "Wait task should wait at least 1500ms, but only waited {:?}",
        elapsed
    );
    assert!(
        elapsed.as_millis() < 2000,
        "Wait task should not wait much longer than 1500ms, but waited {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_wait_task_in_sequence() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with multiple wait tasks in sequence
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-sequence
  version: '1.0.0'
do:
  - wait1:
      wait: PT1S
  - wait2:
      wait:
        milliseconds: 500
  - wait3:
      wait: PT0.5S
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Measure execution time
    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Wait tasks should complete successfully");

    // Total wait time: 1s + 500ms + 0.5s = 2 seconds
    assert!(
        elapsed.as_secs() >= 2,
        "Wait tasks should wait at least 2 seconds total, but only waited {:?}",
        elapsed
    );
    assert!(
        elapsed.as_secs() < 3,
        "Wait tasks should not wait much longer than 2 seconds total, but waited {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_wait_task_returns_empty_result() {
    let (engine, _temp_dir) = setup_test_engine().await;

    // Create workflow with wait task
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-wait-result
  version: '1.0.0'
do:
  - waitTask:
      wait: PT0.1S
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    let result = engine.start_with_input(workflow, json!({})).await;

    assert!(result.is_ok(), "Wait task should complete successfully");

    let (_instance_id, output) = result.unwrap();

    // Wait task should return empty result or minimal metadata
    // The exact output format can be adjusted, but it should be successful
    assert!(
        output.is_object() || output.is_null(),
        "Wait task should return an object or null, got: {:?}",
        output
    );
}