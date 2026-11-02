//! The YAML interpreter's view of the engine's store: with leases on, its writes for an instance go through only while
//! this engine holds the instance's lease ([`LeaseFence`]); with a payload codec, its payloads are encoded
//! ([`CodecStore`]).

use super::Clock;
use crate::persistence::{Error, JournalRecord, Lease, PersistenceProvider, Result};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Wraps the engine's store for the YAML interpreter, which writes to it directly: each write for an instance first
/// makes sure this engine holds the instance's lease (renewing it once less than half is left, as native instances
/// do), and is rejected when another engine took the instance over. Reads, and writes that belong to no instance (the
/// engine's own records, whose ids start with `__jackdaw`), pass through.
pub(crate) struct LeaseFence {
    pub inner: Arc<dyn PersistenceProvider>,
    pub owner: String,
    pub ttl: Duration,
    pub clock: Arc<dyn Clock>,
    /// Until when this engine is known to hold each instance's lease.
    pub held: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl std::fmt::Debug for LeaseFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeaseFence")
            .field("inner", &self.inner)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl LeaseFence {
    async fn check(&self, id: &str) -> Result<()> {
        let id = id.strip_suffix("#journal").unwrap_or(id);
        if id.starts_with("__jackdaw") {
            return Ok(());
        }
        let now = self.clock.now();
        let half = chrono::Duration::from_std(self.ttl / 2).unwrap_or(chrono::Duration::MAX);
        let fresh = super::runtime::lock(&self.held)
            .get(id)
            .is_some_and(|until| *until - now > half);
        if fresh {
            return Ok(());
        }
        let until = super::clock::after(now, self.ttl);
        let lease = self
            .inner
            .acquire_lease(id, &self.owner, now, until)
            .await?;
        if lease.owner == self.owner {
            super::runtime::lock(&self.held).insert(id.to_string(), lease.expires_at);
            Ok(())
        } else {
            super::runtime::lock(&self.held).remove(id);
            Err(Error::Persistence {
                message: format!(
                    "instance `{id}` is now run by engine `{}`: this engine lost its lease, so the write was refused",
                    lease.owner
                ),
            })
        }
    }
}

#[async_trait]
impl PersistenceProvider for LeaseFence {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        self.check(event.instance_id()).await?;
        self.inner.save_event(event).await
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        self.inner.get_events(instance_id).await
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        self.check(&checkpoint.instance_id).await?;
        self.inner.save_checkpoint(checkpoint).await
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        self.inner.get_checkpoint(instance_id).await
    }

    async fn append_journal(&self, instance_id: &str, records: Vec<JournalRecord>) -> Result<()> {
        self.check(instance_id).await?;
        self.inner.append_journal(instance_id, records).await
    }

    async fn load_journal(&self, instance_id: &str) -> Result<Vec<JournalRecord>> {
        self.inner.load_journal(instance_id).await
    }

    async fn prune_journal(&self, instance_id: &str, before: u64) -> Result<()> {
        self.check(instance_id).await?;
        self.inner.prune_journal(instance_id, before).await
    }

    async fn create_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<bool> {
        self.check(&checkpoint.instance_id).await?;
        self.inner.create_checkpoint(checkpoint).await
    }

    async fn list_instances(&self) -> Result<Option<Vec<String>>> {
        self.inner.list_instances().await
    }

    async fn delete_instance(&self, instance_id: &str) -> Result<()> {
        self.inner.delete_instance(instance_id).await
    }

    fn supports_leases(&self) -> bool {
        self.inner.supports_leases()
    }

    async fn acquire_lease(
        &self,
        instance_id: &str,
        owner: &str,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Lease> {
        self.inner
            .acquire_lease(instance_id, owner, now, until)
            .await
    }

    async fn release_lease(&self, instance_id: &str, owner: &str) -> Result<()> {
        super::runtime::lock(&self.held).remove(instance_id);
        self.inner.release_lease(instance_id, owner).await
    }

    async fn post_message(&self, instance_id: &str, message: serde_json::Value) -> Result<()> {
        self.inner.post_message(instance_id, message).await
    }

    async fn take_messages(&self, owner: &str) -> Result<Vec<(String, serde_json::Value)>> {
        self.inner.take_messages(owner).await
    }
}

/// Wraps the engine's store for the YAML interpreter when the engine has a [`PayloadCodec`](super::PayloadCodec): the
/// payloads of its events (inputs, task results, outputs, published events) and its checkpoints' data are encoded on
/// the way in and decoded on the way out, as the engine's own records are. Records written without the codec still
/// read (they carry no codec envelope).
pub(crate) struct CodecStore {
    pub inner: Arc<dyn PersistenceProvider>,
    pub codec: Arc<dyn super::PayloadCodec>,
}

impl std::fmt::Debug for CodecStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodecStore")
            .field("inner", &self.inner)
            .field("codec", &self.codec.name())
            .finish()
    }
}

fn codec_error(error: super::Error) -> Error {
    Error::Persistence {
        message: error.to_string(),
    }
}

impl CodecStore {
    fn encode(&self, value: &serde_json::Value) -> Result<serde_json::Value> {
        super::codec::encode(Some(self.codec.as_ref()), value).map_err(codec_error)
    }

    fn decode(&self, value: serde_json::Value) -> Result<serde_json::Value> {
        super::codec::decode(Some(self.codec.as_ref()), value).map_err(codec_error)
    }

    fn encode_checkpoint(&self, mut checkpoint: WorkflowCheckpoint) -> Result<WorkflowCheckpoint> {
        checkpoint.data = self.encode(&checkpoint.data)?;
        Ok(checkpoint)
    }
}

#[async_trait]
impl PersistenceProvider for CodecStore {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        let event = super::runtime::map_payloads(event, |value| {
            super::codec::encode(Some(self.codec.as_ref()), &value)
        })
        .map_err(codec_error)?;
        self.inner.save_event(event).await
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        self.inner
            .get_events(instance_id)
            .await?
            .into_iter()
            .map(|event| {
                super::runtime::map_payloads(event, |value| {
                    super::codec::decode(Some(self.codec.as_ref()), value)
                })
                .map_err(codec_error)
            })
            .collect()
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        let checkpoint = self.encode_checkpoint(checkpoint)?;
        self.inner.save_checkpoint(checkpoint).await
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        match self.inner.get_checkpoint(instance_id).await? {
            Some(mut checkpoint) => {
                checkpoint.data = self.decode(checkpoint.data)?;
                Ok(Some(checkpoint))
            }
            None => Ok(None),
        }
    }

    async fn append_journal(&self, instance_id: &str, records: Vec<JournalRecord>) -> Result<()> {
        self.inner.append_journal(instance_id, records).await
    }

    async fn load_journal(&self, instance_id: &str) -> Result<Vec<JournalRecord>> {
        self.inner.load_journal(instance_id).await
    }

    async fn prune_journal(&self, instance_id: &str, before: u64) -> Result<()> {
        self.inner.prune_journal(instance_id, before).await
    }

    async fn create_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<bool> {
        let checkpoint = self.encode_checkpoint(checkpoint)?;
        self.inner.create_checkpoint(checkpoint).await
    }

    async fn list_instances(&self) -> Result<Option<Vec<String>>> {
        self.inner.list_instances().await
    }

    async fn delete_instance(&self, instance_id: &str) -> Result<()> {
        self.inner.delete_instance(instance_id).await
    }

    fn supports_leases(&self) -> bool {
        self.inner.supports_leases()
    }

    async fn acquire_lease(
        &self,
        instance_id: &str,
        owner: &str,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Lease> {
        self.inner
            .acquire_lease(instance_id, owner, now, until)
            .await
    }

    async fn release_lease(&self, instance_id: &str, owner: &str) -> Result<()> {
        self.inner.release_lease(instance_id, owner).await
    }

    async fn post_message(&self, instance_id: &str, message: serde_json::Value) -> Result<()> {
        self.inner.post_message(instance_id, message).await
    }

    async fn take_messages(&self, owner: &str) -> Result<Vec<(String, serde_json::Value)>> {
        self.inner.take_messages(owner).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::providers::persistence::InMemoryPersistence;
    use crate::testing::ManualClock;

    fn event(id: &str) -> WorkflowEvent {
        WorkflowEvent::WorkflowResumed {
            instance_id: id.to_string(),
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn writes_stop_once_another_engine_takes_the_lease() {
        let store: Arc<dyn PersistenceProvider> = Arc::new(InMemoryPersistence::new());
        let clock = ManualClock::new();
        let fence = |owner: &str| LeaseFence {
            inner: Arc::clone(&store),
            owner: owner.to_string(),
            ttl: Duration::from_secs(30),
            clock: Arc::new(clock.clone()),
            held: Mutex::new(HashMap::new()),
        };
        let (one, two) = (fence("one"), fence("two"));

        one.save_event(event("i-1"))
            .await
            .expect("the first writer takes the lease");
        two.save_event(event("i-1"))
            .await
            .expect_err("the lease is held by the first engine");

        // The first engine stalls past its lease; the second takes the instance over.
        clock.advance(Duration::from_secs(31));
        two.save_event(event("i-1"))
            .await
            .expect("an expired lease can be taken");
        let refused = one
            .save_event(event("i-1"))
            .await
            .expect_err("the stalled engine's write");
        assert!(refused.to_string().contains("engine `two`"), "{refused}");
        assert_eq!(store.get_events("i-1").await.unwrap().len(), 2);

        // The engine's own records belong to no instance.
        one.save_event(event("__jackdaw_emitted__"))
            .await
            .expect("not fenced");
    }
}
