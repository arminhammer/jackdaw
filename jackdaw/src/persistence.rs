use crate::workflow::WorkflowCheckpoint;
use crate::workflow::WorkflowEvent;
use async_trait::async_trait;
use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
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

/// One entry of an instance's append-only journal log: its position and its JSON body.
pub type JournalRecord = (u64, serde_json::Value);

/// The checkpoint id under which the default journal-log methods keep an instance's log.
fn journal_key(instance_id: &str) -> String {
    format!("{instance_id}#journal")
}

/// Where workflow state lives: events (history), checkpoints (the latest state of an instance) and, for Rust
/// workflows, an append-only journal log per instance.
///
/// Only the first four methods are required. The journal-log methods have default implementations on top of
/// checkpoints, which are correct but rewrite the whole log on every append; a store that can append and range-scan
/// should override them (all built-in stores do), and override [`list_instances`](Self::list_instances) and
/// [`delete_instance`](Self::delete_instance) to support listing and purging.
#[async_trait]
pub trait PersistenceProvider: Send + Sync + std::fmt::Debug {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()>;
    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>>;
    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()>;
    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>>;

    /// Appends `records` to the instance's journal log, in one write. Positions only grow.
    async fn append_journal(&self, instance_id: &str, records: Vec<JournalRecord>) -> Result<()> {
        let mut all = self.load_journal(instance_id).await?;
        all.extend(records);
        self.save_checkpoint(WorkflowCheckpoint {
            instance_id: journal_key(instance_id),
            current_task: String::new(),
            data: serde_json::to_value(&all).context(SerializationSnafu)?,
            timestamp: chrono::Utc::now(),
        })
        .await
    }

    /// The instance's journal log in position order.
    async fn load_journal(&self, instance_id: &str) -> Result<Vec<JournalRecord>> {
        match self.get_checkpoint(&journal_key(instance_id)).await? {
            Some(checkpoint) => serde_json::from_value(checkpoint.data).context(SerializationSnafu),
            None => Ok(Vec::new()),
        }
    }

    /// Removes the instance's journal records before position `before` (all of them for `u64::MAX`).
    async fn prune_journal(&self, instance_id: &str, before: u64) -> Result<()> {
        let kept: Vec<JournalRecord> = self
            .load_journal(instance_id)
            .await?
            .into_iter()
            .filter(|(position, _)| *position >= before)
            .collect();
        self.save_checkpoint(WorkflowCheckpoint {
            instance_id: journal_key(instance_id),
            current_task: String::new(),
            data: serde_json::to_value(&kept).context(SerializationSnafu)?,
            timestamp: chrono::Utc::now(),
        })
        .await
    }

    /// The ids of every instance with a checkpoint, or `None` if this store cannot list them (the engine then keeps an
    /// index of its own).
    async fn list_instances(&self) -> Result<Option<Vec<String>>> {
        Ok(None)
    }

    /// Removes an instance's checkpoint, events and journal log.
    async fn delete_instance(&self, instance_id: &str) -> Result<()> {
        Err(Error::Persistence {
            message: format!(
                "this store cannot delete instances (asked to delete `{instance_id}`)"
            ),
        })
    }
}
