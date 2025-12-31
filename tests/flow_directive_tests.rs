#![allow(clippy::unwrap_used)]

/// Tests for flow control directives (exit, end, continue)
use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::json;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::sync::Arc;

#[tokio::test]
async fn test_exit_directive_terminates_workflow() {
    // Setup
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

    // Define a workflow that uses "exit" directive
    let workflow_yaml = r#" 
document:
  dsl: '1.0.2'
  namespace: default
  name: test-exit-directive
  version: '1.0.0'
do:
  - firstTask:
      set:
        step: 1
        colors: []
  - switchTask:
      switch:
        - case1:
            when: '.step == 1'
            then: exit
        - case2:
            when: '.step == 2'
            then: shouldNotRun
  - shouldNotRun:
      set:
        colors: '${ .colors + ["should_not_run"] }'
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Execute workflow
    let result = engine.start_with_input(workflow, json!({})).await;

    // Assert: workflow should complete successfully
    assert!(result.is_ok(), "Workflow should complete successfully");

    let (_instance_id, output) = result.unwrap();

    // The workflow should exit after switchTask, so shouldNotRun should not have executed
    assert_eq!(
        output.get("step").and_then(|v| v.as_i64()),
        Some(1),
        "Workflow should exit before shouldNotRun task, so step should still be 1"
    );

    // Verify colors array is empty (shouldNotRun was not executed)
    assert_eq!(
        output
            .get("colors")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0),
        "colors array should be empty since shouldNotRun was not executed"
    );
}

#[tokio::test]
async fn test_end_directive_terminates_workflow() {
    // Setup
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

    // Define a workflow that uses "end" directive
    let workflow_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-end-directive
  version: '1.0.0'
do:
  - firstTask:
      set:
        step: 1
        colors: []
  - switchTask:
      switch:
        - case1:
            when: '.step == 1'
            then: end
        - case2:
            when: '.step == 2'
            then: shouldNotRun
  - shouldNotRun:
      set:
        colors: '${ .colors + ["should_not_run"] }'
"#;

    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml).unwrap();

    // Execute workflow
    let result = engine.start_with_input(workflow, json!({})).await;

    // Assert: workflow should complete successfully
    assert!(result.is_ok(), "Workflow should complete successfully");

    let (_instance_id, output) = result.unwrap();

    // The workflow should end after switchTask, so shouldNotRun should not have executed
    assert_eq!(
        output.get("step").and_then(|v| v.as_i64()),
        Some(1),
        "Workflow should end before shouldNotRun task, so step should still be 1"
    );

    // Verify colors array is empty (shouldNotRun was not executed)
    assert_eq!(
        output
            .get("colors")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0),
        "colors array should be empty since shouldNotRun was not executed"
    );
}

#[tokio::test]
async fn test_exit_vs_end_behavior_identical_in_main_scope() {
    // This test verifies that in the main workflow scope (not nested),
    // both "exit" and "end" behave identically and terminate the workflow

    // Setup for exit test
    let temp_dir_exit = tempfile::tempdir().unwrap();
    let db_path_exit = temp_dir_exit.path().join("test_exit.db");
    let persistence_exit = Arc::new(RedbPersistence::new(db_path_exit.to_str().unwrap()).unwrap());
    let cache_exit = Arc::new(RedbCache::new(Arc::clone(&persistence_exit.db)).unwrap())
        as Arc<dyn CacheProvider>;
    let engine_exit = Arc::new(
        DurableEngine::new(
            Arc::clone(&persistence_exit) as Arc<dyn PersistenceProvider>,
            Arc::clone(&cache_exit),
        )
        .unwrap(),
    );

    // Setup for end test
    let temp_dir_end = tempfile::tempdir().unwrap();
    let db_path_end = temp_dir_end.path().join("test_end.db");
    let persistence_end = Arc::new(RedbPersistence::new(db_path_end.to_str().unwrap()).unwrap());
    let cache_end = Arc::new(RedbCache::new(Arc::clone(&persistence_end.db)).unwrap())
        as Arc<dyn CacheProvider>;
    let engine_end = Arc::new(
        DurableEngine::new(
            Arc::clone(&persistence_end) as Arc<dyn PersistenceProvider>,
            Arc::clone(&cache_end),
        )
        .unwrap(),
    );

    let workflow_exit_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-exit
  version: '1.0.0'
do:
  - task1:
      set:
        value: 'exit'
      then: exit
  - task2:
      set:
        value: 'should not run'
"#;

    let workflow_end_yaml = r#"
document:
  dsl: '1.0.2'
  namespace: default
  name: test-end
  version: '1.0.0'
do:
  - task1:
      set:
        value: 'end'
      then: end
  - task2:
      set:
        value: 'should not run'
"#;

    let workflow_exit: WorkflowDefinition = serde_yaml::from_str(workflow_exit_yaml).unwrap();
    let workflow_end: WorkflowDefinition = serde_yaml::from_str(workflow_end_yaml).unwrap();

    // Execute both workflows
    let (_instance_id_exit, output_exit) = engine_exit
        .start_with_input(workflow_exit, json!({}))
        .await
        .unwrap();
    let (_instance_id_end, output_end) = engine_end
        .start_with_input(workflow_end, json!({}))
        .await
        .unwrap();

    // Both should have the same output structure - both should terminate after task1
    assert_eq!(output_exit.get("value"), Some(&json!("exit")));
    assert_eq!(output_end.get("value"), Some(&json!("end")));
}
