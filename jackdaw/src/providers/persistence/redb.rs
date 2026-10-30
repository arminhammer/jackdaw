use async_trait::async_trait;
use chrono::Utc;
use snafu::prelude::*;
use std::sync::Arc;

use crate::persistence::{Error, JournalRecord, PersistenceProvider, Result, SerializationSnafu};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};
use redb::ReadableTable as _;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct RedbPersistence {
    pub db: Arc<redb::Database>,
}

pub const EVENTS_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("events");
pub const CHECKPOINTS_TABLE: redb::TableDefinition<&str, &[u8]> =
    redb::TableDefinition::new("checkpoints");
/// Journal logs, keyed `"{instance}\u{1f}{position:020}"` so one instance's records are one ordered range.
pub const JOURNAL_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("journal");

/// Makes event keys unique within a process even when two events share a timestamp.
static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn db_error(what: &str, error: impl std::fmt::Display) -> Error {
    Error::Database {
        message: format!("{what}: {error}"),
    }
}

fn journal_key(instance_id: &str, position: u64) -> String {
    format!("{instance_id}\u{1f}{position:020}")
}

/// The key range holding every record of `instance_id`.
fn journal_range(instance_id: &str) -> (String, String) {
    (
        format!("{instance_id}\u{1f}"),
        format!("{instance_id}\u{20}"),
    )
}

impl RedbPersistence {
    /// Creates a new `RedbPersistence` instance with the database at the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The database cannot be created at the given path
    /// - A write transaction cannot be started
    /// - The events or checkpoints tables cannot be opened
    /// - The transaction cannot be committed
    pub fn new(path: &str) -> Result<Self> {
        let db = redb::Database::create(path).map_err(|e| Error::Database {
            message: format!("Failed to create database: {e}"),
        })?;
        let write_txn = db.begin_write().map_err(|e| Error::Database {
            message: format!("Failed to begin write transaction: {e}"),
        })?;
        {
            write_txn
                .open_table(EVENTS_TABLE)
                .map_err(|e| Error::Database {
                    message: format!("Failed to open events table: {e}"),
                })?;
            write_txn
                .open_table(CHECKPOINTS_TABLE)
                .map_err(|e| Error::Database {
                    message: format!("Failed to open checkpoints table: {e}"),
                })?;
            write_txn
                .open_table(JOURNAL_TABLE)
                .map_err(|e| db_error("Failed to open journal table", e))?;
        }
        write_txn.commit().map_err(|e| Error::Database {
            message: format!("Failed to commit transaction: {e}"),
        })?;
        Ok(Self { db: Arc::new(db) })
    }
}

#[async_trait]
impl PersistenceProvider for RedbPersistence {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write().map_err(|e| Error::Database {
                message: format!("Failed to begin write transaction: {e}"),
            })?;
            {
                let mut table =
                    write_txn
                        .open_table(EVENTS_TABLE)
                        .map_err(|e| Error::Database {
                            message: format!("Failed to open events table: {e}"),
                        })?;
                // Time first so events sort in order; the counter keeps two events in one nanosecond apart.
                let key = format!(
                    "{}:{:020}:{:010}",
                    event.instance_id(),
                    Utc::now().timestamp_nanos_opt().unwrap_or(0),
                    EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
                );
                let value = serde_json::to_vec(&event).context(SerializationSnafu)?;
                table
                    .insert(key.as_str(), value.as_slice())
                    .map_err(|e| Error::Database {
                        message: format!("Failed to insert event: {e}"),
                    })?;
            }
            write_txn.commit().map_err(|e| Error::Database {
                message: format!("Failed to commit transaction: {e}"),
            })?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Database {
            message: format!("Task join error: {e}"),
        })?
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<WorkflowEvent>> {
            let read_txn = db.begin_read().map_err(|e| Error::Database {
                message: format!("Failed to begin read transaction: {e}"),
            })?;
            let table = read_txn
                .open_table(EVENTS_TABLE)
                .map_err(|e| Error::Database {
                    message: format!("Failed to open events table: {e}"),
                })?;
            let mut events = Vec::new();
            // Only this instance's keys: `{id}:` up to (not including) `{id};`.
            let (start, end) = (format!("{instance_id}:"), format!("{instance_id};"));
            let range = table
                .range::<&str>(start.as_str()..end.as_str())
                .map_err(|e| Error::Database {
                    message: format!("Failed to create range: {e}"),
                })?;
            for item in range {
                let (_, value) = item.map_err(|e| Error::Database {
                    message: format!("Failed to read item: {e}"),
                })?;
                let event: WorkflowEvent =
                    serde_json::from_slice(value.value()).context(SerializationSnafu)?;
                events.push(event);
            }
            Ok(events)
        })
        .await
        .map_err(|e| Error::Database {
            message: format!("Task join error: {e}"),
        })?
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write().map_err(|e| Error::Database {
                message: format!("Failed to begin write transaction: {e}"),
            })?;
            {
                let mut table =
                    write_txn
                        .open_table(CHECKPOINTS_TABLE)
                        .map_err(|e| Error::Database {
                            message: format!("Failed to open checkpoints table: {e}"),
                        })?;
                let value = serde_json::to_vec(&checkpoint).context(SerializationSnafu)?;
                table
                    .insert(checkpoint.instance_id.as_str(), value.as_slice())
                    .map_err(|e| Error::Database {
                        message: format!("Failed to insert checkpoint: {e}"),
                    })?;
            }
            write_txn.commit().map_err(|e| Error::Database {
                message: format!("Failed to commit transaction: {e}"),
            })?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Database {
            message: format!("Task join error: {e}"),
        })?
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<WorkflowCheckpoint>> {
            let read_txn = db.begin_read().map_err(|e| Error::Database {
                message: format!("Failed to begin read transaction: {e}"),
            })?;
            let table = read_txn
                .open_table(CHECKPOINTS_TABLE)
                .map_err(|e| Error::Database {
                    message: format!("Failed to open checkpoints table: {e}"),
                })?;
            if let Some(value) = table
                .get(instance_id.as_str())
                .map_err(|e| Error::Database {
                    message: format!("Failed to get checkpoint: {e}"),
                })?
            {
                let checkpoint: WorkflowCheckpoint =
                    serde_json::from_slice(value.value()).context(SerializationSnafu)?;
                Ok(Some(checkpoint))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| Error::Database {
            message: format!("Task join error: {e}"),
        })?
    }

    async fn append_journal(&self, instance_id: &str, records: Vec<JournalRecord>) -> Result<()> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db
                .begin_write()
                .map_err(|e| db_error("Failed to begin write transaction", e))?;
            {
                let mut table = write_txn
                    .open_table(JOURNAL_TABLE)
                    .map_err(|e| db_error("Failed to open journal table", e))?;
                for (position, body) in records {
                    let value = serde_json::to_vec(&body).context(SerializationSnafu)?;
                    table
                        .insert(
                            journal_key(&instance_id, position).as_str(),
                            value.as_slice(),
                        )
                        .map_err(|e| db_error("Failed to append a journal record", e))?;
                }
            }
            write_txn
                .commit()
                .map_err(|e| db_error("Failed to commit transaction", e))
        })
        .await
        .map_err(|e| db_error("Task join error", e))?
    }

    async fn load_journal(&self, instance_id: &str) -> Result<Vec<JournalRecord>> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<JournalRecord>> {
            let read_txn = db
                .begin_read()
                .map_err(|e| db_error("Failed to begin read transaction", e))?;
            let table = read_txn
                .open_table(JOURNAL_TABLE)
                .map_err(|e| db_error("Failed to open journal table", e))?;
            let (start, end) = journal_range(&instance_id);
            let mut records = Vec::new();
            for item in table
                .range::<&str>(start.as_str()..end.as_str())
                .map_err(|e| db_error("Failed to create range", e))?
            {
                let (key, value) =
                    item.map_err(|e| db_error("Failed to read a journal record", e))?;
                let position = key
                    .value()
                    .rsplit('\u{1f}')
                    .next()
                    .and_then(|digits| digits.parse().ok())
                    .unwrap_or(0);
                let body = serde_json::from_slice(value.value()).context(SerializationSnafu)?;
                records.push((position, body));
            }
            Ok(records)
        })
        .await
        .map_err(|e| db_error("Task join error", e))?
    }

    async fn prune_journal(&self, instance_id: &str, before: u64) -> Result<()> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db
                .begin_write()
                .map_err(|e| db_error("Failed to begin write transaction", e))?;
            {
                let mut table = write_txn
                    .open_table(JOURNAL_TABLE)
                    .map_err(|e| db_error("Failed to open journal table", e))?;
                let start = journal_range(&instance_id).0;
                let end = if before == u64::MAX {
                    journal_range(&instance_id).1
                } else {
                    journal_key(&instance_id, before)
                };
                table
                    .retain_in::<&str, _>(start.as_str()..end.as_str(), |_, _| false)
                    .map_err(|e| db_error("Failed to prune the journal", e))?;
            }
            write_txn
                .commit()
                .map_err(|e| db_error("Failed to commit transaction", e))
        })
        .await
        .map_err(|e| db_error("Task join error", e))?
    }

    async fn create_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<bool> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let write_txn = db
                .begin_write()
                .map_err(|e| db_error("Failed to begin write transaction", e))?;
            let created = {
                let mut table = write_txn
                    .open_table(CHECKPOINTS_TABLE)
                    .map_err(|e| db_error("Failed to open checkpoints table", e))?;
                let exists = table
                    .get(checkpoint.instance_id.as_str())
                    .map_err(|e| db_error("Failed to read a checkpoint", e))?
                    .is_some();
                if !exists {
                    let value = serde_json::to_vec(&checkpoint).context(SerializationSnafu)?;
                    table
                        .insert(checkpoint.instance_id.as_str(), value.as_slice())
                        .map_err(|e| db_error("Failed to insert checkpoint", e))?;
                }
                !exists
            };
            write_txn
                .commit()
                .map_err(|e| db_error("Failed to commit transaction", e))?;
            Ok(created)
        })
        .await
        .map_err(|e| db_error("Task join error", e))?
    }

    async fn list_instances(&self) -> Result<Option<Vec<String>>> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<Vec<String>>> {
            let read_txn = db
                .begin_read()
                .map_err(|e| db_error("Failed to begin read transaction", e))?;
            let table = read_txn
                .open_table(CHECKPOINTS_TABLE)
                .map_err(|e| db_error("Failed to open checkpoints table", e))?;
            let mut ids = Vec::new();
            for item in table
                .range::<&str>(..)
                .map_err(|e| db_error("Failed to create range", e))?
            {
                let (key, _) = item.map_err(|e| db_error("Failed to read a checkpoint", e))?;
                ids.push(key.value().to_string());
            }
            Ok(Some(ids))
        })
        .await
        .map_err(|e| db_error("Task join error", e))?
    }

    async fn delete_instance(&self, instance_id: &str) -> Result<()> {
        let db = self.db.clone();
        let instance_id = instance_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db
                .begin_write()
                .map_err(|e| db_error("Failed to begin write transaction", e))?;
            {
                let mut checkpoints = write_txn
                    .open_table(CHECKPOINTS_TABLE)
                    .map_err(|e| db_error("Failed to open checkpoints table", e))?;
                checkpoints
                    .remove(instance_id.as_str())
                    .map_err(|e| db_error("Failed to delete the checkpoint", e))?;
                let mut events = write_txn
                    .open_table(EVENTS_TABLE)
                    .map_err(|e| db_error("Failed to open events table", e))?;
                let (start, end) = (format!("{instance_id}:"), format!("{instance_id};"));
                events
                    .retain_in::<&str, _>(start.as_str()..end.as_str(), |_, _| false)
                    .map_err(|e| db_error("Failed to delete events", e))?;
                let mut journal = write_txn
                    .open_table(JOURNAL_TABLE)
                    .map_err(|e| db_error("Failed to open journal table", e))?;
                let (start, end) = journal_range(&instance_id);
                journal
                    .retain_in::<&str, _>(start.as_str()..end.as_str(), |_, _| false)
                    .map_err(|e| db_error("Failed to delete the journal", e))?;
            }
            write_txn
                .commit()
                .map_err(|e| db_error("Failed to commit transaction", e))
        })
        .await
        .map_err(|e| db_error("Task join error", e))?
    }
}
