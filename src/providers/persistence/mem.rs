use crate::persistence::{PersistenceProvider, Result};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct InMemoryPersistence {
    events: Arc<Mutex<HashMap<String, Vec<WorkflowEvent>>>>,
    checkpoints: Arc<Mutex<HashMap<String, WorkflowCheckpoint>>>,
}

impl Default for InMemoryPersistence {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(HashMap::new())),
            checkpoints: Arc::new(Mutex::new(HashMap::new())),
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

        events
            .entry(instance_id)
            .or_insert_with(Vec::new)
            .push(event);

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
}
