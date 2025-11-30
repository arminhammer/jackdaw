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

#[derive(Clone)]
pub struct Context {
    pub instance_id: String,
    pub data: Data,
    pub persistence: Arc<dyn PersistenceProvider>,
    pub cache: Arc<dyn CacheProvider>,
    pub history: Arc<ExecutionHistory>,
    pub current_task: Arc<RwLock<String>>,
    pub workflow: Arc<WorkflowDefinition>,
    pub next_task: Arc<RwLock<Option<String>>>,
    pub initial_input: Arc<serde_json::Value>,
    pub data_modified: Arc<RwLock<bool>>,
    pub task_output_keys: Arc<RwLock<HashSet<String>>>,
    pub scalar_output_tasks: Arc<RwLock<HashSet<String>>>,
    // Current task input (used for $input in expressions)
    // Tracks the previous task's output for sequential task execution
    pub task_input: Arc<RwLock<serde_json::Value>>,
    // Descriptors for expression evaluation
    pub runtime_descriptor: Arc<RuntimeDescriptor>,
    pub workflow_descriptor: Arc<WorkflowDescriptor>,
    // Task index for color-coding streaming output in concurrent scenarios
    pub task_index: Option<usize>,
}

impl Context {
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
            instance_id,
            data: Arc::new(RwLock::new(data_with_descriptors.clone())),
            persistence,
            cache,
            history,
            current_task: Arc::new(RwLock::new(current_task)),
            workflow: Arc::new(workflow.clone()),
            next_task: Arc::new(RwLock::new(None)),
            initial_input: Arc::new(initial_data.clone()),
            data_modified: Arc::new(RwLock::new(false)),
            task_output_keys: Arc::new(RwLock::new(HashSet::new())),
            scalar_output_tasks: Arc::new(RwLock::new(HashSet::new())),
            task_input: Arc::new(RwLock::new(data_with_descriptors)),
            runtime_descriptor: Arc::new(runtime_descriptor),
            workflow_descriptor: Arc::new(workflow_descriptor),
            task_index: None,
        })
    }

    pub async fn merge(&self, key: &str, value: serde_json::Value) {
        let mut data = self.data.write().await;
        if let Some(obj) = data.as_object_mut() {
            obj.insert(key.to_string(), value);
            *self.data_modified.write().await = true;
            // Track that this key was set by a task
            self.task_output_keys.write().await.insert(key.to_string());
        } else {
            // If data is not an object (e.g., after input filtering to a scalar),
            // replace it with a new object containing the key-value pair
            let mut new_obj = serde_json::Map::new();
            new_obj.insert(key.to_string(), value);
            *data = serde_json::Value::Object(new_obj);
            *self.data_modified.write().await = true;
            self.task_output_keys.write().await.insert(key.to_string());
        }
    }

    pub async fn save_checkpoint(&self, task_name: &str) -> Result<()> {
        let data = self.data.read().await;
        self.persistence
            .save_checkpoint(WorkflowCheckpoint {
                instance_id: self.instance_id.clone(),
                current_task: task_name.to_string(),
                data: data.clone(),
                timestamp: Utc::now(),
            })
            .await
            .context(PersistenceSnafu)
    }
}
