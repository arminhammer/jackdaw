use chrono::Utc;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

type Data = Arc<RwLock<serde_json::Value>>;

use crate::cache::CacheProvider;
use crate::descriptors::{RuntimeDescriptor, WorkflowDescriptor};
use crate::executionhistory::ExecutionHistory;
use crate::persistence::PersistenceProvider;
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("No tasks in workflow"))]
    NoTasks,

    #[snafu(display("Persistence error: {source}"))]
    Persistence { source: crate::persistence::Error },

    #[snafu(display("Serialization error: {source}"))]
    Serialization { source: serde_json::Error },

    #[snafu(display("Context error: {message}"))]
    Context { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Execution state that changes during workflow execution
#[derive(Clone)]
pub struct ExecutionState {
    pub data: Data,
    pub task_input: Arc<RwLock<serde_json::Value>>,
    pub current_task: Arc<RwLock<String>>,
    pub next_task: Arc<RwLock<Option<String>>>,
    pub task_index: Option<usize>,
    /// Cancellation flag - when set to true, workflow execution should stop
    pub cancelled: Arc<RwLock<bool>>,
    /// Cancellation reason - optional message explaining why workflow was cancelled
    pub cancellation_reason: Arc<RwLock<Option<String>>>,
    /// Suspension flag - when set to true, workflow should pause and save state
    pub suspended: Arc<RwLock<bool>>,
    /// Suspension reason - optional message explaining why workflow was suspended
    pub suspension_reason: Arc<RwLock<Option<String>>>,
}

/// Static workflow metadata (immutable during execution)
#[derive(Clone)]
#[allow(dead_code)]
pub struct WorkflowMetadata {
    pub instance_id: String,
    pub workflow: Arc<WorkflowDefinition>,
    pub initial_input: Arc<serde_json::Value>,
    pub runtime_descriptor: Arc<RuntimeDescriptor>,
    pub workflow_descriptor: Arc<WorkflowDescriptor>,
}

/// External services for I/O operations
#[derive(Clone)]
pub struct ExecutionServices {
    pub persistence: Arc<dyn PersistenceProvider>,
    pub cache: Arc<dyn CacheProvider>,
    pub history: Arc<ExecutionHistory>,
}

/// Tracking metadata (could potentially be eliminated or simplified)
#[derive(Clone)]
#[allow(dead_code)]
pub struct ExecutionTracking {
    pub data_modified: Arc<RwLock<bool>>,
    pub task_output_keys: Arc<RwLock<HashSet<String>>>,
    pub scalar_output_tasks: Arc<RwLock<HashSet<String>>>,
}

/// Main context - composition of focused structures
#[derive(Clone)]
#[allow(dead_code)]
pub struct Context {
    pub state: ExecutionState,
    pub metadata: WorkflowMetadata,
    pub services: ExecutionServices,
    pub tracking: ExecutionTracking,
}

impl Context {
    /// Creates a new context for workflow execution.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The workflow has no tasks defined
    /// - There is a persistence error when retrieving events or checkpoints
    /// - Serialization of workflow descriptors fails
    pub async fn new(
        workflow: &WorkflowDefinition,
        persistence: Arc<dyn PersistenceProvider>,
        cache: Arc<dyn CacheProvider>,
        instance_id: Option<String>,
        initial_data: serde_json::Value,
    ) -> Result<Self> {
        let instance_id = instance_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let events = persistence
            .get_events(&instance_id)
            .await
            .context(PersistenceSnafu)?;
        let history = Arc::new(ExecutionHistory::new(&events));

        let (data, current_task) = if let Some(checkpoint) = persistence
            .get_checkpoint(&instance_id)
            .await
            .context(PersistenceSnafu)?
        {
            (checkpoint.data, checkpoint.current_task)
        } else {
            let first_task_name = workflow
                .do_
                .entries
                .first()
                .and_then(|map| map.keys().next())
                .ok_or(Error::NoTasks)?
                .clone();

            persistence
                .save_event(WorkflowEvent::WorkflowStarted {
                    instance_id: instance_id.clone(),
                    workflow_id: workflow.document.name.clone(),
                    timestamp: Utc::now(),
                    initial_data: initial_data.clone(),
                })
                .await
                .context(PersistenceSnafu)?;

            (initial_data.clone(), first_task_name)
        };

        // Create runtime descriptor
        let runtime_descriptor =
            RuntimeDescriptor::new("jackdaw".to_string(), env!("CARGO_PKG_VERSION").to_string());

        // Create workflow descriptor
        let workflow_started_at = Utc::now();
        let workflow_descriptor = WorkflowDescriptor::new(
            instance_id.clone(),
            serde_json::to_value(workflow).context(SerializationSnafu)?,
            initial_data.clone(),
            workflow_started_at,
        );

        // Inject descriptors into data for expression evaluation
        let data_with_descriptors = if let serde_json::Value::Object(mut obj) = data {
            obj.insert(
                "__workflow".to_string(),
                serde_json::to_value(&workflow_descriptor).context(SerializationSnafu)?,
            );
            obj.insert(
                "__runtime".to_string(),
                serde_json::to_value(&runtime_descriptor).context(SerializationSnafu)?,
            );
            serde_json::Value::Object(obj)
        } else {
            data
        };

        Ok(Self {
            state: ExecutionState {
                data: Arc::new(RwLock::new(data_with_descriptors.clone())),
                task_input: Arc::new(RwLock::new(data_with_descriptors)),
                current_task: Arc::new(RwLock::new(current_task)),
                next_task: Arc::new(RwLock::new(None)),
                task_index: None,
                cancelled: Arc::new(RwLock::new(false)),
                cancellation_reason: Arc::new(RwLock::new(None)),
                suspended: Arc::new(RwLock::new(false)),
                suspension_reason: Arc::new(RwLock::new(None)),
            },
            metadata: WorkflowMetadata {
                instance_id,
                workflow: Arc::new(workflow.clone()),
                initial_input: Arc::new(initial_data.clone()),
                runtime_descriptor: Arc::new(runtime_descriptor),
                workflow_descriptor: Arc::new(workflow_descriptor),
            },
            services: ExecutionServices {
                persistence,
                cache,
                history,
            },
            tracking: ExecutionTracking {
                data_modified: Arc::new(RwLock::new(false)),
                task_output_keys: Arc::new(RwLock::new(HashSet::new())),
                scalar_output_tasks: Arc::new(RwLock::new(HashSet::new())),
            },
        })
    }

    pub async fn merge(&self, key: &str, value: serde_json::Value) {
        let mut data = self.state.data.write().await;
        if let Some(obj) = data.as_object_mut() {
            obj.insert(key.to_string(), value);
            *self.tracking.data_modified.write().await = true;
            // Track that this key was set by a task
            self.tracking
                .task_output_keys
                .write()
                .await
                .insert(key.to_string());
        } else {
            // If data is not an object (e.g., after input filtering to a scalar),
            // replace it with a new object containing the key-value pair
            let mut new_obj = serde_json::Map::new();
            new_obj.insert(key.to_string(), value);
            *data = serde_json::Value::Object(new_obj);
            *self.tracking.data_modified.write().await = true;
            self.tracking
                .task_output_keys
                .write()
                .await
                .insert(key.to_string());
        }
    }

    /// Saves the current workflow execution state as a checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error if there is a persistence error when saving the checkpoint.
    pub async fn save_checkpoint(&self, task_name: &str) -> Result<()> {
        let data = self.state.data.read().await;
        self.services
            .persistence
            .save_checkpoint(WorkflowCheckpoint {
                instance_id: self.metadata.instance_id.clone(),
                current_task: task_name.to_string(),
                data: data.clone(),
                timestamp: Utc::now(),
            })
            .await
            .context(PersistenceSnafu)
    }

    /// Cancel the workflow execution
    ///
    /// # Errors
    ///
    /// Returns an error if there is a persistence error when saving the cancellation event.
    #[allow(dead_code)]
    pub async fn cancel(&self, reason: Option<String>) -> Result<()> {
        // Set cancellation flags
        *self.state.cancelled.write().await = true;
        *self.state.cancellation_reason.write().await = reason.clone();

        // Emit WorkflowCancelled event
        self.services
            .persistence
            .save_event(WorkflowEvent::WorkflowCancelled {
                instance_id: self.metadata.instance_id.clone(),
                reason,
                timestamp: Utc::now(),
            })
            .await
            .context(PersistenceSnafu)
    }

    /// Suspend the workflow execution
    ///
    /// # Errors
    ///
    /// Returns an error if there is a persistence error when saving the suspension event or checkpoint.
    #[allow(dead_code)]
    pub async fn suspend(&self, reason: Option<String>) -> Result<()> {
        // Set suspension flags
        *self.state.suspended.write().await = true;
        *self.state.suspension_reason.write().await = reason.clone();

        // Save current state as checkpoint
        let current_task = self.state.current_task.read().await.clone();
        self.save_checkpoint(&current_task).await?;

        // Emit WorkflowSuspended event with checkpoint data
        let data = self.state.data.read().await.clone();
        self.services
            .persistence
            .save_event(WorkflowEvent::WorkflowSuspended {
                instance_id: self.metadata.instance_id.clone(),
                reason,
                checkpoint_data: data,
                timestamp: Utc::now(),
            })
            .await
            .context(PersistenceSnafu)
    }

    /// Resume the workflow execution from suspended state
    ///
    /// # Errors
    ///
    /// Returns an error if there is a persistence error when saving the resumption event.
    #[allow(dead_code)]
    pub async fn resume(&self) -> Result<()> {
        // Clear suspension flags
        *self.state.suspended.write().await = false;
        *self.state.suspension_reason.write().await = None;

        // Emit WorkflowResumed event
        self.services
            .persistence
            .save_event(WorkflowEvent::WorkflowResumed {
                instance_id: self.metadata.instance_id.clone(),
                timestamp: Utc::now(),
            })
            .await
            .context(PersistenceSnafu)
    }

    /// Check if workflow is cancelled
    pub async fn is_cancelled(&self) -> bool {
        *self.state.cancelled.read().await
    }

    /// Check if workflow is suspended
    pub async fn is_suspended(&self) -> bool {
        *self.state.suspended.read().await
    }
}
