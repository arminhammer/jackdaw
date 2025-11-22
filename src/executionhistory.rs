use std::collections::HashMap;

use crate::workflow::WorkflowEvent;

#[derive(Clone)]
pub struct ExecutionHistory {
    completed_tasks: HashMap<String, serde_json::Value>,
}

impl ExecutionHistory {
    pub fn new(events: Vec<WorkflowEvent>) -> Self {
        let mut completed_tasks = HashMap::new();
        for event in &events {
            if let WorkflowEvent::TaskCompleted {
                task_name, result, ..
            } = event
            {
                completed_tasks.insert(task_name.clone(), result.clone());
            }
        }
        Self { completed_tasks }
    }

    pub fn is_task_completed(&self, task_name: &str) -> Option<&serde_json::Value> {
        self.completed_tasks.get(task_name)
    }
}
