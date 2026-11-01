//! Where workflow state lives: the [`PersistenceProvider`] trait. The built-in stores are in
//! [`providers::persistence`](crate::providers::persistence); [`api::Store`](crate::api::Store) wraps them for
//! [`EngineBuilder::store`](crate::api::EngineBuilder::store).
use crate::workflow::WorkflowCheckpoint;
use crate::workflow::WorkflowEvent;
use async_trait::async_trait;
use snafu::prelude::*;

/// Why a store operation failed.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
#[non_exhaustive]
pub enum Error {
    /// The store refused the operation (or does not support it).
    #[snafu(display("Persistence error: {message}"))]
    Persistence {
        /// What went wrong.
        message: String,
    },

    /// The database behind the store failed.
    #[snafu(display("Database error: {message}"))]
    Database {
        /// What went wrong.
        message: String,
    },

    /// A record could not be (de)serialized.
    #[snafu(display("Serialization error: {source}"))]
    Serialization {
        /// The underlying error.
        source: serde_json::Error,
    },

    /// No events exist for the instance.
    #[snafu(display("Event not found: {instance_id}"))]
    EventNotFound {
        /// The instance.
        instance_id: String,
    },

    /// No checkpoint exists for the instance.
    #[snafu(display("Checkpoint not found: {instance_id}"))]
    CheckpointNotFound {
        /// The instance.
        instance_id: String,
    },
}

/// The result of a store operation.
pub type Result<T> = std::result::Result<T, Error>;

/// Which engine owns an instance, and until when (see [`PersistenceProvider::acquire_lease`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Lease {
    /// The owning engine's id.
    pub owner: String,
    /// When the lease runs out unless it is renewed.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

fn leases_unsupported() -> Error {
    Error::Persistence {
        message: "this store does not support leases".to_string(),
    }
}

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
    /// Appends an event to the instance's history.
    async fn save_event(&self, event: WorkflowEvent) -> Result<()>;
    /// The instance's history, oldest first.
    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>>;
    /// Stores the instance's latest checkpoint, replacing the previous one.
    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()>;
    /// The instance's latest checkpoint, if any.
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

    /// Saves `checkpoint` only if there is none for its instance yet, and says whether it did. Built-in stores do this
    /// atomically, which is what keeps two engines from starting the same instance; the default reads then writes, so
    /// a store shared by several engines (or one that wraps another) should override it.
    async fn create_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<bool> {
        if self
            .get_checkpoint(&checkpoint.instance_id)
            .await?
            .is_some()
        {
            return Ok(false);
        }
        self.save_checkpoint(checkpoint).await?;
        Ok(true)
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

    /// Whether this store keeps leases. With leases, several engines (in one process or many) can share the store:
    /// each instance is run by the engine that holds its lease. A store without them can have one engine at a time.
    fn supports_leases(&self) -> bool {
        false
    }

    /// Takes the lease on `instance_id` for `owner` until `until`, or renews it, unless another owner holds a lease
    /// that has not expired at `now`. Returns the lease as it stands afterwards: its owner is `owner` if the call took
    /// it. Must be atomic.
    async fn acquire_lease(
        &self,
        instance_id: &str,
        owner: &str,
        now: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<Lease> {
        let _ = (instance_id, owner, now, until);
        Err(leases_unsupported())
    }

    /// Gives up `owner`'s lease on `instance_id`. Does nothing if another owner holds it.
    async fn release_lease(&self, instance_id: &str, owner: &str) -> Result<()> {
        let _ = (instance_id, owner);
        Ok(())
    }

    /// Leaves a message (a signal, an update) for an instance that another engine runs.
    async fn post_message(&self, instance_id: &str, message: serde_json::Value) -> Result<()> {
        let _ = (instance_id, message);
        Err(leases_unsupported())
    }

    /// Removes and returns, oldest first, the messages for instances whose lease `owner` holds.
    async fn take_messages(&self, owner: &str) -> Result<Vec<(String, serde_json::Value)>> {
        let _ = owner;
        Ok(Vec::new())
    }
}
