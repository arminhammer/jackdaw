//! The records a store keeps: history events and checkpoints (see [`PersistenceProvider`](crate::PersistenceProvider)).
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One entry of an instance's history, as stores keep it. [`api::Event`](crate::api::Event) is the library's view of
/// these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowEvent {
    /// An instance started.
    WorkflowStarted {
        /// The instance.
        instance_id: String,
        /// The workflow's identity.
        workflow_id: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
        /// The input.
        initial_data: serde_json::Value,
    },
    /// A YAML task was entered.
    TaskEntered {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A YAML task was created.
    TaskCreated {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// The kind of task (`call`, `set`, ...).
        task_type: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task started.
    TaskStarted {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task failed and is being retried.
    TaskRetried {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// The attempt that failed, starting at 1.
        attempt: u32,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task finished successfully.
    TaskCompleted {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// What the task returned.
        result: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
        /// How long it took, in milliseconds.
        duration_ms: i64,
    },
    /// The instance finished successfully.
    WorkflowCompleted {
        /// The instance.
        instance_id: String,
        /// The output.
        final_data: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
        /// How long it took, in milliseconds.
        duration_ms: i64,
    },
    /// Emitted when a workflow starts correlating events
    ///
    /// Spec-compliant event following the Serverless Workflow specification.
    WorkflowCorrelationStarted {
        /// The instance.
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
        /// The instance.
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
    /// The instance failed.
    WorkflowFailed {
        /// The instance.
        instance_id: String,
        /// A description of the failure.
        error: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// The instance was cancelled.
    WorkflowCancelled {
        /// The instance.
        instance_id: String,
        /// Why, if given.
        reason: Option<String>,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// The instance was suspended.
    WorkflowSuspended {
        /// The instance.
        instance_id: String,
        /// Why, if given.
        reason: Option<String>,
        /// The state to resume from.
        checkpoint_data: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// The instance was resumed.
    WorkflowResumed {
        /// The instance.
        instance_id: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task was cancelled.
    TaskCancelled {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// Why, if given.
        reason: Option<String>,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task was suspended.
    TaskSuspended {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// The task's state.
        state: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task was resumed.
    TaskResumed {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task failed (after any retries).
    TaskFaulted {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// A description of the failure.
        error: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A Rust workflow ended its run with `ctx.continue_as_new(input)` and started run `run` with `input`.
    WorkflowContinuedAsNew {
        /// The instance.
        instance_id: String,
        /// The number of the run that starts (the first is 1).
        run: u32,
        /// The new run's input.
        input: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A Rust workflow published an event with `ctx.emit` (the event as a CloudEvent JSON object).
    EventEmitted {
        /// The instance.
        instance_id: String,
        /// The event (a CloudEvent JSON object).
        event: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
}

impl WorkflowEvent {
    /// The instance the event belongs to.
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
            | WorkflowEvent::TaskFaulted { instance_id, .. }
            | WorkflowEvent::WorkflowContinuedAsNew { instance_id, .. }
            | WorkflowEvent::EventEmitted { instance_id, .. } => instance_id,
        }
    }
}

/// The latest state of an instance, replaced on every write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    /// The instance.
    pub instance_id: String,
    /// The last task or step recorded.
    pub current_task: String,
    /// The instance's state (for Rust workflows, the journal header).
    pub data: serde_json::Value,
    /// When it happened.
    pub timestamp: DateTime<Utc>,
}
