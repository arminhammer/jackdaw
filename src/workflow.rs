use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    TaskCreated {
        instance_id: String,
        task_name: String,
        task_type: String,
        timestamp: DateTime<Utc>,
    },
    TaskStarted {
        instance_id: String,
        task_name: String,
        timestamp: DateTime<Utc>,
    },
    TaskRetried {
        instance_id: String,
        task_name: String,
        attempt: u32,
        timestamp: DateTime<Utc>,
    },
    TaskCompleted {
        instance_id: String,
        task_name: String,
        result: serde_json::Value,
        timestamp: DateTime<Utc>,
        duration_ms: i64,
    },
    WorkflowCompleted {
        instance_id: String,
        final_data: serde_json::Value,
        timestamp: DateTime<Utc>,
        duration_ms: i64,
    },
    /// Emitted when a workflow starts correlating events
    ///
    /// Spec-compliant event following the Serverless Workflow specification.
    WorkflowCorrelationStarted {
        instance_id: String,
        /// The qualified name of the workflow instance
        name: String,
        /// The date and time at which correlation started
        started_at: DateTime<Utc>,
    },
    /// Emitted when a workflow completes correlating events
    ///
    /// Spec-compliant event following the Serverless Workflow specification,
    /// with a jackdaw extension field `correlation_output` for library convenience.
    WorkflowCorrelationCompleted {
        instance_id: String,
        /// The qualified name of the workflow instance
        name: String,
        /// The ID of the correlation context (identifies which correlation completed)
        correlation_context: String,
        /// A key/value mapping of the correlation keys
        correlation_keys: HashMap<String, String>,
        /// The date and time at which correlation completed
        completed_at: DateTime<Utc>,
        /// **Jackdaw extension**: The output data from processing this correlation
        ///
        /// This is a non-spec extension field that makes it convenient for library
        /// users to get the result of each correlation/iteration without querying
        /// workflow instance state.
        correlation_output: Option<serde_json::Value>,
    },
    WorkflowFailed {
        instance_id: String,
        error: String,
        timestamp: DateTime<Utc>,
    },
    WorkflowCancelled {
        instance_id: String,
        reason: Option<String>,
        timestamp: DateTime<Utc>,
    },
    WorkflowSuspended {
        instance_id: String,
        reason: Option<String>,
        checkpoint_data: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
    WorkflowResumed {
        instance_id: String,
        timestamp: DateTime<Utc>,
    },
    TaskCancelled {
        instance_id: String,
        task_name: String,
        reason: Option<String>,
        timestamp: DateTime<Utc>,
    },
    TaskSuspended {
        instance_id: String,
        task_name: String,
        state: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
    TaskResumed {
        instance_id: String,
        task_name: String,
        timestamp: DateTime<Utc>,
    },
    TaskFaulted {
        instance_id: String,
        task_name: String,
        error: String,
        timestamp: DateTime<Utc>,
    },
}

impl WorkflowEvent {
    #[must_use]
    pub fn instance_id(&self) -> &str {
        match self {
            WorkflowEvent::WorkflowStarted { instance_id, .. }
            | WorkflowEvent::TaskEntered { instance_id, .. }
            | WorkflowEvent::TaskCreated { instance_id, .. }
            | WorkflowEvent::TaskStarted { instance_id, .. }
            | WorkflowEvent::TaskRetried { instance_id, .. }
            | WorkflowEvent::TaskCompleted { instance_id, .. }
            | WorkflowEvent::WorkflowCompleted { instance_id, .. }
            | WorkflowEvent::WorkflowCorrelationStarted { instance_id, .. }
            | WorkflowEvent::WorkflowCorrelationCompleted { instance_id, .. }
            | WorkflowEvent::WorkflowFailed { instance_id, .. }
            | WorkflowEvent::WorkflowCancelled { instance_id, .. }
            | WorkflowEvent::WorkflowSuspended { instance_id, .. }
            | WorkflowEvent::WorkflowResumed { instance_id, .. }
            | WorkflowEvent::TaskCancelled { instance_id, .. }
            | WorkflowEvent::TaskSuspended { instance_id, .. }
            | WorkflowEvent::TaskResumed { instance_id, .. }
            | WorkflowEvent::TaskFaulted { instance_id, .. } => instance_id,
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
