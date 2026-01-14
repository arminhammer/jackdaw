#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::json;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Helper to set up test infrastructure
async fn setup_test_engine() -> (Arc<DurableEngine>, tempfile::TempDir, Arc<RedbPersistence>) {
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
    (engine, temp_dir, persistence)
}

/// Helper to load workflow from fixture
fn load_workflow(fixture_path: &str) -> WorkflowDefinition {
    let path = PathBuf::from(fixture_path);
    let workflow_yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", fixture_path));
    serde_yaml::from_str(&workflow_yaml)
        .unwrap_or_else(|_| panic!("Failed to parse workflow: {}", fixture_path))
}

// ====================================================================================
// EVERY SCHEDULE TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_every_seconds() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-seconds.sw.yaml");

    // Start the workflow - it should detect the schedule and run the scheduler
    // This call should block and run indefinitely, so we run it in a LocalSet with timeout
    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            // Run for 5 seconds then abort
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .ok();
        })
        .await;

    // TDD: Count how many workflow instances were started
    // With 'every: 2s' schedule running for 5 seconds, should execute at: 0s, 2s, 4s = 3 times

    // Get all events from persistence and count WorkflowStarted events
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert_eq!(
        execution_count, 3,
        "Expected 3 workflow executions (at 0s, 2s, 4s), but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_every_milliseconds() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-milliseconds.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_millis(1600), handle)
                .await
                .ok();
        })
        .await;

    // Should execute at 0ms, 500ms, 1000ms (at least 3 times in 1.6 seconds)
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 3,
        "Expected at least 3 executions in 1.6s with 500ms interval, but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_every_composite() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-composite.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_millis(3600), handle)
                .await
                .ok();
        })
        .await;

    // Composite duration: 1s + 500ms = 1.5s interval
    // In 3.6s should execute at: 0s, 1.5s, 3s = 3 times
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert_eq!(
        execution_count, 3,
        "Expected 3 executions with 1.5s composite interval, but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_every_iso8601() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-iso8601.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .ok();
        })
        .await;

    // ISO 8601 format: PT2S = 2 seconds
    // In 5s should execute at: 0s, 2s, 4s = 3 times
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert_eq!(
        execution_count, 3,
        "Expected 3 executions with PT2S (ISO 8601) interval, but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_every_with_long_task() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-with-long-task.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(4), handle)
                .await
                .ok();
        })
        .await;

    // Task takes 2s but schedule is every 1s
    // Should start instances at: 0s, 1s, 2s, 3s = 4 instances (overlapping execution)
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 4,
        "Expected at least 4 concurrent instances (every 1s despite 2s task duration), but found {}",
        execution_count
    );
}

// ====================================================================================
// CRON SCHEDULE TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_cron_parsing() {
    let workflow = load_workflow("tests/fixtures/schedules/schedule-cron-every-minute.sw.yaml");

    // Verify workflow parses successfully with schedule field
    assert!(workflow.schedule.is_some());
    let schedule = workflow.schedule.as_ref().unwrap();
    assert!(schedule.cron.is_some());
    assert_eq!(schedule.cron.as_ref().unwrap(), "* * * * *");
}

#[tokio::test]
#[ignore] // This would take too long - run manually if needed
async fn test_schedule_cron_execution() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-cron-every-5-seconds.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(12), handle)
                .await
                .ok();
        })
        .await;

    // Cron: every 5 seconds, running for 12s should give ~2-3 executions
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 2,
        "Expected at least 2 executions with cron schedule, but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_cron_daily_midnight() {
    let workflow = load_workflow("tests/fixtures/schedules/schedule-cron-daily-midnight.sw.yaml");

    assert!(workflow.schedule.is_some());
    let schedule = workflow.schedule.as_ref().unwrap();
    assert!(schedule.cron.is_some());
    assert_eq!(schedule.cron.as_ref().unwrap(), "0 0 * * *");

    // TODO: Once scheduler is implemented, verify:
    // - Next execution time is calculated as next midnight
    // - Can query scheduler for next_execution_time
}

// ====================================================================================
// AFTER SCHEDULE TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_after_seconds() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-after-seconds.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .ok();
        })
        .await;

    // 'after' waits for completion + 2s before next execution
    // Timeline: execute(~instant), wait(2s), execute(~instant), wait(2s), execute
    // In 5s should complete ~3 executions
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 2,
        "Expected at least 2-3 executions with 'after: 2s' schedule, but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_after_iso8601() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-after-iso8601.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(3), handle)
                .await
                .ok();
        })
        .await;

    // 'after' with PT1S (1 second delay after completion)
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 2,
        "Expected at least 2-3 executions with 'after: PT1S', but found {}",
        execution_count
    );
}

#[tokio::test]
async fn test_schedule_after_with_task_delay() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-after-with-delay.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(3), handle)
                .await
                .ok();
        })
        .await;

    // Workflow has internal 500ms wait, then 'after: 1s' delay
    // Cycle time = 500ms execution + 1s delay = 1.5s per cycle
    // In 3s should complete 2 full cycles
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert_eq!(
        execution_count, 2,
        "Expected 2 executions (1.5s cycle time in 3s), but found {}",
        execution_count
    );
}

// ====================================================================================
// VALIDATION TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_invalid_multiple_types() {
    let workflow = load_workflow("tests/fixtures/schedules/schedule-invalid-multiple-types.sw.yaml");

    // Workflow parses, but should fail validation
    assert!(workflow.schedule.is_some());

    // TODO: Once validation is implemented in validate.rs:
    // let result = validate_schedule(&workflow.schedule.unwrap());
    // assert!(result.is_err());
    // assert!(result.unwrap_err().to_string().contains("only one schedule type"));
}

#[tokio::test]
async fn test_schedule_invalid_cron_expression() {
    let workflow = load_workflow("tests/fixtures/schedules/schedule-invalid-cron-expression.sw.yaml");

    assert!(workflow.schedule.is_some());

    // TODO: Once validation is implemented:
    // let result = validate_schedule(&workflow.schedule.unwrap());
    // assert!(result.is_err());
    // assert!(result.unwrap_err().to_string().contains("Invalid cron"));
}

#[tokio::test]
async fn test_schedule_no_schedule_runs_once() {
    let (engine, _temp_dir) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-no-schedule.sw.yaml");

    assert!(workflow.schedule.is_none());

    let start = Instant::now();
    let result = engine.start_with_input(workflow, json!({})).await;
    let elapsed = start.elapsed();

    // Should complete immediately (no schedule = single execution)
    assert!(result.is_ok());
    assert!(
        elapsed.as_millis() < 100,
        "Workflow without schedule should execute once and complete quickly, took {:?}",
        elapsed
    );

    // Verify it ran exactly once
    let (instance_id, _output) = result.unwrap();
    assert!(!instance_id.is_empty());
}

// ====================================================================================
// EVENT-DRIVEN SCHEDULE TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_event_driven_one() {
    let workflow = load_workflow("tests/fixtures/schedules/schedule-event-driven-one.sw.yaml");

    assert!(workflow.schedule.is_some());
    let schedule = workflow.schedule.as_ref().unwrap();
    assert!(schedule.on.is_some());

    // TODO: Event-driven schedules should delegate to Listen tasks
    // Verify that:
    // 1. start_with_input recognizes event-driven schedule
    // 2. Falls back to listener-based execution
    // 3. Workflow waits for events rather than time-based triggers
}

// ====================================================================================
// SIGNAL HANDLING & CANCELLATION TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_graceful_shutdown() {
    let (engine, _temp_dir, persistence) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-seconds.sw.yaml");

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_millis(500), handle)
                .await
                .ok();
        })
        .await;

    // Give it time to clean up
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Scheduler should have started at least one execution before being aborted
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 1,
        "Expected at least 1 execution before abort, but found {}",
        execution_count
    );
}

// ====================================================================================
// ERROR HANDLING TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_continues_after_workflow_error() {
    // TODO: Create fixture with workflow that raises an error
    // Verify that:
    // 1. Workflow error is logged
    // 2. Scheduler continues running
    // 3. Next scheduled execution still occurs
    // 4. Error doesn't terminate the scheduler
}

#[tokio::test]
async fn test_schedule_with_workflow_timeout() {
    // TODO: Create fixture with schedule + workflow timeout
    // Verify that:
    // 1. Individual workflow execution can timeout
    // 2. Timeout emits WorkflowTimeout event
    // 3. Scheduler continues after timeout
    // 4. Next scheduled execution still occurs
}

// ====================================================================================
// PERSISTENCE & RECOVERY TESTS
// ====================================================================================

#[tokio::test]
async fn test_schedule_query_execution_history() {
    let (engine, temp_dir) = setup_test_engine().await;
    let workflow = load_workflow("tests/fixtures/schedules/schedule-every-seconds.sw.yaml");

    let _persistence = Arc::new(
        RedbPersistence::new(temp_dir.path().join("test.db").to_str().unwrap()).unwrap(),
    );

    let engine_clone = Arc::clone(&engine);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = tokio::task::spawn_local(async move {
                engine_clone.start_with_input(workflow, json!({})).await
            });

            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .ok();
        })
        .await;

    // Verify we can query persistence for execution history
    let all_instance_ids = persistence.list_instance_ids().await.unwrap();
    let execution_count = all_instance_ids.len();

    assert!(
        execution_count >= 3,
        "Expected at least 3 executions in 5s with 2s interval, but found {}. \
         Persistence querying should work.",
        execution_count
    );
}

// ====================================================================================
// HELPER FUNCTIONS (to be implemented with scheduler)
// ====================================================================================

/// Helper to count workflow executions from persistence
/// Returns: number of unique workflow instances started
#[allow(dead_code)]
async fn count_workflow_executions(_persistence: &Arc<dyn PersistenceProvider>) -> usize {
    // TODO: Implement when WorkflowEvent querying is available
    // persistence.get_events()
    //     .filter(WorkflowStarted)
    //     .map(|e| e.instance_id)
    //     .unique()
    //     .count()
    0
}

/// Helper to get execution timestamps
/// Returns: Vec of (instance_id, timestamp) for all workflow starts
#[allow(dead_code)]
async fn get_execution_timestamps(
    _persistence: &Arc<dyn PersistenceProvider>,
) -> Vec<(String, chrono::DateTime<chrono::Utc>)> {
    // TODO: Implement when WorkflowEvent querying is available
    vec![]
}

/// Helper to verify execution timing within tolerance
#[allow(dead_code)]
fn verify_timing_tolerance(
    _timestamps: &[(String, chrono::DateTime<chrono::Utc>)],
    _expected_interval_ms: i64,
    _tolerance_ms: i64,
) -> bool {
    // TODO: Check that intervals between executions are within tolerance
    true
}
