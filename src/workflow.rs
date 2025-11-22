use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowEvent {
    WorkflowStarted {
        instance_id: String,
        workflow_id: String,
        timestamp: DateTime<Utc>,
        initial_data: serde_json::Value,
    },
    TaskEntered {
        instance_id: String,
        task_name: String,
        timestamp: DateTime<Utc>,
    },
    TaskStarted {
        instance_id: String,
        task_name: String,
        timestamp: DateTime<Utc>,
    },
    TaskCompleted {
        instance_id: String,
        task_name: String,
        result: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
    WorkflowCompleted {
        instance_id: String,
        final_data: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
    WorkflowFailed {
        instance_id: String,
        error: String,
        timestamp: DateTime<Utc>,
    },
}

impl WorkflowEvent {
    pub fn instance_id(&self) -> &str {
        match self {
            WorkflowEvent::WorkflowStarted { instance_id, .. }
            | WorkflowEvent::TaskEntered { instance_id, .. }
            | WorkflowEvent::TaskStarted { instance_id, .. }
            | WorkflowEvent::TaskCompleted { instance_id, .. }
            | WorkflowEvent::WorkflowCompleted { instance_id, .. }
            | WorkflowEvent::WorkflowFailed { instance_id, .. } => instance_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    pub instance_id: String,
    pub current_task: String,
    pub data: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}
