use crate::workflow::WorkflowCheckpoint;
use crate::workflow::WorkflowEvent;
use async_trait::async_trait;
use snafu::prelude::*;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Persistence error: {message}"))]
    Persistence { message: String },

    #[snafu(display("Database error: {message}"))]
    Database { message: String },

    #[snafu(display("Serialization error: {source}"))]
    Serialization { source: serde_json::Error },

    #[snafu(display("Event not found: {instance_id}"))]
    EventNotFound { instance_id: String },

    #[snafu(display("Checkpoint not found: {instance_id}"))]
    CheckpointNotFound { instance_id: String },
}

pub type Result<T> = std::result::Result<T, Error>;

#[async_trait]
pub trait PersistenceProvider: Send + Sync + std::fmt::Debug {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()>;
    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>>;
    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()>;
    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>>;
}
