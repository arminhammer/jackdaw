use crate::CtKWorld;

pub use cucumber::{World, given, then, when};
pub use jackdaw::workflow::WorkflowEvent;

// Helper function to get the timestamp when a task started or completed
fn get_task_timestamp(
    events: &[WorkflowEvent],
    task_name: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    for event in events {
        match event {
            WorkflowEvent::TaskStarted {
                task_name: name,
                timestamp,
                ..
            } if name == task_name => {
                return Some(*timestamp);
            }
            WorkflowEvent::TaskCompleted {
                task_name: name,
                timestamp,
                ..
            } if name == task_name => {
                return Some(*timestamp);
            }
            _ => {}
        }
    }
    None
}

// Check that a specific task ran first (earliest timestamp)
#[then(expr = "{word} should run first")]
async fn then_task_runs_first(world: &mut CtKWorld, task_name: String) {
    let task_timestamp = get_task_timestamp(&world.workflow_events, &task_name).expect(&format!(
        "Task '{}' was not found in workflow events",
        task_name
    ));

    // Find all task timestamps
    let mut all_task_timestamps: Vec<(String, chrono::DateTime<chrono::Utc>)> = Vec::new();

    for event in &world.workflow_events {
        match event {
            WorkflowEvent::TaskStarted {
                task_name: name,
                timestamp,
                ..
            } => {
                all_task_timestamps.push((name.clone(), timestamp.clone()));
            }
            _ => {}
        }
    }

    // Verify this task has the earliest timestamp
    for (name, timestamp) in all_task_timestamps {
        if name != task_name && timestamp < task_timestamp {
            panic!(
                "Task '{}' did not run first. Task '{}' started at {:?}, but '{}' started at {:?}",
                task_name, name, timestamp, task_name, task_timestamp
            );
        }
    }
}

// Check that one task runs after another
#[then(expr = "{word} should run after {word}")]
async fn then_task_runs_after(world: &mut CtKWorld, task_name: String, predecessor: String) {
    let task_timestamp = get_task_timestamp(&world.workflow_events, &task_name).expect(&format!(
        "Task '{}' was not found in workflow events",
        task_name
    ));

    let predecessor_timestamp = get_task_timestamp(&world.workflow_events, &predecessor).expect(
        &format!("Task '{}' was not found in workflow events", predecessor),
    );

    assert!(
        task_timestamp > predecessor_timestamp,
        "Task '{}' (at {:?}) should run after '{}' (at {:?}), but it didn't",
        task_name,
        task_timestamp,
        predecessor,
        predecessor_timestamp
    );
}
