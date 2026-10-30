use crate::persistence::{JournalRecord, PersistenceProvider, Result};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct InMemoryPersistence {
    events: Arc<Mutex<HashMap<String, Vec<WorkflowEvent>>>>,
    checkpoints: Arc<Mutex<HashMap<String, WorkflowCheckpoint>>>,
    journals: Arc<Mutex<HashMap<String, Vec<JournalRecord>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Default for InMemoryPersistence {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(HashMap::new())),
            checkpoints: Arc::new(Mutex::new(HashMap::new())),
            journals: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl InMemoryPersistence {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PersistenceProvider for InMemoryPersistence {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        let instance_id = event.instance_id().to_string();
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        events.entry(instance_id).or_default().push(event);

        Ok(())
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        let events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        Ok(events.get(instance_id).cloned().unwrap_or_default())
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        let instance_id = checkpoint.instance_id.clone();
        let mut checkpoints = self
            .checkpoints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        checkpoints.insert(instance_id, checkpoint);

        Ok(())
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        let checkpoints = self
            .checkpoints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        Ok(checkpoints.get(instance_id).cloned())
    }

    async fn append_journal(&self, instance_id: &str, records: Vec<JournalRecord>) -> Result<()> {
        lock(&self.journals)
            .entry(instance_id.to_string())
            .or_default()
            .extend(records);
        Ok(())
    }

    async fn load_journal(&self, instance_id: &str) -> Result<Vec<JournalRecord>> {
        Ok(lock(&self.journals)
            .get(instance_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn prune_journal(&self, instance_id: &str, before: u64) -> Result<()> {
        if let Some(records) = lock(&self.journals).get_mut(instance_id) {
            records.retain(|(position, _)| *position >= before);
        }
        Ok(())
    }

    async fn create_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<bool> {
        let mut checkpoints = lock(&self.checkpoints);
        if checkpoints.contains_key(&checkpoint.instance_id) {
            return Ok(false);
        }
        checkpoints.insert(checkpoint.instance_id.clone(), checkpoint);
        Ok(true)
    }

    async fn list_instances(&self) -> Result<Option<Vec<String>>> {
        Ok(Some(lock(&self.checkpoints).keys().cloned().collect()))
    }

    async fn delete_instance(&self, instance_id: &str) -> Result<()> {
        lock(&self.checkpoints).remove(instance_id);
        lock(&self.events).remove(instance_id);
        lock(&self.journals).remove(instance_id);
        Ok(())
    }
}
