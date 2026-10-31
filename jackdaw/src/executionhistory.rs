use std::collections::HashMap;

use crate::workflow::WorkflowEvent;

#[derive(Clone)]
pub struct ExecutionHistory {
    completed_tasks: HashMap<String, serde_json::Value>,
    /// When each task last started, before this run of the instance.
    started: HashMap<String, chrono::DateTime<chrono::Utc>>,
}

impl ExecutionHistory {
    #[must_use]
    pub fn new(events: &[WorkflowEvent]) -> Self {
        let mut completed_tasks = HashMap::new();
        let mut started = HashMap::new();
        for event in events {
            if let WorkflowEvent::TaskCompleted {
                task_name, result, ..
            } = event
            {
                completed_tasks.insert(task_name.clone(), result.clone());
            } else if let WorkflowEvent::TaskStarted {
                task_name,
                timestamp,
                ..
            } = event
            {
                started.insert(task_name.clone(), *timestamp);
            }
        }
        Self {
            completed_tasks,
            started,
        }
    }

    /// When `task_name` started in an earlier run of the instance (one that was interrupted), if it did.
    #[must_use]
    pub fn started_at(&self, task_name: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        self.started.get(task_name).copied()
    }

    #[must_use]
    pub fn is_task_completed(&self, task_name: &str) -> Option<&serde_json::Value> {
        self.completed_tasks.get(task_name)
    }
}
