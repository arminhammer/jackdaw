use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;

use crate::persistence::{PersistenceProvider, Result, Error};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};

#[derive(Debug)]
pub struct RedbPersistence {
    pub db: Arc<redb::Database>,
}

pub const EVENTS_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("events");
pub const CHECKPOINTS_TABLE: redb::TableDefinition<&str, &[u8]> =
    redb::TableDefinition::new("checkpoints");

impl RedbPersistence {
    pub fn new(path: &str) -> Result<Self> {
        let db = redb::Database::create(path)
            .map_err(|e| Error::Database { message: format!("Failed to create database: {}", e) })?;
        let write_txn = db.begin_write()
            .map_err(|e| Error::Database { message: format!("Failed to begin write transaction: {}", e) })?;
        {
            write_txn.open_table(EVENTS_TABLE)
                .map_err(|e| Error::Database { message: format!("Failed to open events table: {}", e) })?;
            write_txn.open_table(CHECKPOINTS_TABLE)
                .map_err(|e| Error::Database { message: format!("Failed to open checkpoints table: {}", e) })?;
        }
        write_txn.commit()
            .map_err(|e| Error::Database { message: format!("Failed to commit transaction: {}", e) })?;
        Ok(Self { db: Arc::new(db) })
    }
}

#[async_trait]
impl PersistenceProvider for RedbPersistence {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write()
                .map_err(|e| Error::Database { message: format!("Failed to begin write transaction: {}", e) })?;
            {
                let mut table = write_txn.open_table(EVENTS_TABLE)
                    .map_err(|e| Error::Database { message: format!("Failed to open events table: {}", e) })?;
                let key = format!(
                    "{}:{}",
                    event.instance_id(),
                    Utc::now().timestamp_nanos_opt().unwrap_or(0)
                );
                let value = serde_json::to_vec(&event)
                    .map_err(|e| Error::Serialization { source: e })?;
                table.insert(key.as_str(), value.as_slice())
                    .map_err(|e| Error::Database { message: format!("Failed to insert event: {}", e) })?;
            }
            write_txn.commit()
                .map_err(|e| Error::Database { message: format!("Failed to commit transaction: {}", e) })?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<WorkflowEvent>> {
            let read_txn = db.begin_read()
                .map_err(|e| Error::Database { message: format!("Failed to begin read transaction: {}", e) })?;
            let table = read_txn.open_table(EVENTS_TABLE)
                .map_err(|e| Error::Database { message: format!("Failed to open events table: {}", e) })?;
            let mut events = Vec::new();
            let prefix = format!("{}:", instance_id);
            let range = table.range::<&str>(..)
                .map_err(|e| Error::Database { message: format!("Failed to create range: {}", e) })?;
            for item in range {
                let (key, value) = item
                    .map_err(|e| Error::Database { message: format!("Failed to read item: {}", e) })?;
                if key.value().starts_with(&prefix) {
                    let event: WorkflowEvent = serde_json::from_slice(value.value())
                        .map_err(|e| Error::Serialization { source: e })?;
                    events.push(event);
                }
            }
            Ok(events)
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write()
                .map_err(|e| Error::Database { message: format!("Failed to begin write transaction: {}", e) })?;
            {
                let mut table = write_txn.open_table(CHECKPOINTS_TABLE)
                    .map_err(|e| Error::Database { message: format!("Failed to open checkpoints table: {}", e) })?;
                let value = serde_json::to_vec(&checkpoint)
                    .map_err(|e| Error::Serialization { source: e })?;
                table.insert(checkpoint.instance_id.as_str(), value.as_slice())
                    .map_err(|e| Error::Database { message: format!("Failed to insert checkpoint: {}", e) })?;
            }
            write_txn.commit()
                .map_err(|e| Error::Database { message: format!("Failed to commit transaction: {}", e) })?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<WorkflowCheckpoint>> {
            let read_txn = db.begin_read()
                .map_err(|e| Error::Database { message: format!("Failed to begin read transaction: {}", e) })?;
            let table = read_txn.open_table(CHECKPOINTS_TABLE)
                .map_err(|e| Error::Database { message: format!("Failed to open checkpoints table: {}", e) })?;
            if let Some(value) = table.get(instance_id.as_str())
                .map_err(|e| Error::Database { message: format!("Failed to get checkpoint: {}", e) })? {
                let checkpoint: WorkflowCheckpoint = serde_json::from_slice(value.value())
                    .map_err(|e| Error::Serialization { source: e })?;
                Ok(Some(checkpoint))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }
}
