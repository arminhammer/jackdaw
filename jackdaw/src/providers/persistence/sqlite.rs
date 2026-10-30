//! A store in a SQLite database.
use crate::persistence::{
    Error, JournalRecord, Lease, PersistenceProvider, Result, SerializationSnafu,
};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};
use async_trait::async_trait;
use snafu::prelude::*;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

#[derive(Debug)]
#[allow(dead_code)]
/// A store in a SQLite database. Several processes can share the file.
pub struct SqlitePersistence {
    pool: SqlitePool,
}

#[allow(dead_code)]
impl SqlitePersistence {
    /// Create a new ``SQLite`` persistence provider
    ///
    /// # Arguments
    /// * `database_url` - ``SQLite`` connection string (e.g., "sqlite:workflows.db" or ``"sqlite::memory:"``)
    ///
    /// # Errors
    /// Returns an error if the database connection fails or schema initialization fails.
    ///
    /// # Example
    /// ```no_run
    /// # use jackdaw::providers::persistence::SqlitePersistence;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let persistence = SqlitePersistence::new("sqlite:workflows.db").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to connect to SQLite: {e}"),
            })?;

        // Initialize schema
        sqlx::query(include_str!("./sql/persistence_sqlite.sql"))
            .execute(&pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to execute schema: {e}"),
            })?;

        Ok(Self { pool })
    }

    /// Create a new ``SQLite`` persistence provider with custom pool options
    ///
    /// # Arguments
    /// * `pool` - An initialized `SqlitePool` to use for persistence.
    ///
    /// # Errors
    /// Returns an error if schema initialization fails (e.g., SQL execution errors).
    pub async fn with_pool(pool: SqlitePool) -> Result<Self> {
        // Initialize schema
        sqlx::query(include_str!("./sql/persistence_sqlite.sql"))
            .execute(&pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to execute schema: {e}"),
            })?;

        Ok(Self { pool })
    }

    /// Get the event type name for a ``WorkflowEvent``
    fn get_event_type(event: &WorkflowEvent) -> &'static str {
        match event {
            WorkflowEvent::WorkflowStarted { .. } => "WorkflowStarted",
            WorkflowEvent::TaskEntered { .. } => "TaskEntered",
            WorkflowEvent::TaskCreated { .. } => "TaskCreated",
            WorkflowEvent::TaskStarted { .. } => "TaskStarted",
            WorkflowEvent::TaskRetried { .. } => "TaskRetried",
            WorkflowEvent::TaskCompleted { .. } => "TaskCompleted",
            WorkflowEvent::WorkflowCompleted { .. } => "WorkflowCompleted",
            WorkflowEvent::WorkflowCorrelationStarted { .. } => "WorkflowCorrelationStarted",
            WorkflowEvent::WorkflowCorrelationCompleted { .. } => "WorkflowCorrelationCompleted",
            WorkflowEvent::WorkflowFailed { .. } => "WorkflowFailed",
            WorkflowEvent::WorkflowCancelled { .. } => "WorkflowCancelled",
            WorkflowEvent::WorkflowSuspended { .. } => "WorkflowSuspended",
            WorkflowEvent::WorkflowResumed { .. } => "WorkflowResumed",
            WorkflowEvent::TaskCancelled { .. } => "TaskCancelled",
            WorkflowEvent::TaskSuspended { .. } => "TaskSuspended",
            WorkflowEvent::TaskResumed { .. } => "TaskResumed",
            WorkflowEvent::TaskFaulted { .. } => "TaskFaulted",
            WorkflowEvent::WorkflowContinuedAsNew { .. } => "WorkflowContinuedAsNew",
            WorkflowEvent::EventEmitted { .. } => "EventEmitted",
        }
    }
}

#[async_trait]
impl PersistenceProvider for SqlitePersistence {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        let instance_id = event.instance_id().to_string();
        let event_type = Self::get_event_type(&event);
        let event_data = serde_json::to_string(&event).context(SerializationSnafu)?;
        let timestamp = chrono::Utc::now().to_rfc3339();

        // Get the next sequence number for this instance
        let sequence_number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence_number), -1) + 1 FROM workflow_events WHERE instance_id = ?"
        )
        .bind(&instance_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to get sequence number: {e}") })?;

        sqlx::query(
            "INSERT INTO workflow_events (instance_id, event_type, event_data, timestamp, sequence_number) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&instance_id)
        .bind(event_type)
        .bind(&event_data)
        .bind(&timestamp)
        .bind(sequence_number)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to save event: {e}") })?;

        Ok(())
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT event_data FROM workflow_events WHERE instance_id = ? ORDER BY sequence_number ASC"
        )
        .bind(instance_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to get events: {e}") })?;

        let mut events = Vec::new();
        for (event_data,) in rows {
            let event: WorkflowEvent =
                serde_json::from_str(&event_data).context(SerializationSnafu)?;
            events.push(event);
        }

        Ok(events)
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        let data_json = serde_json::to_string(&checkpoint.data).context(SerializationSnafu)?;
        let timestamp_str = checkpoint.timestamp.to_rfc3339();

        sqlx::query(
            "INSERT OR REPLACE INTO workflow_checkpoints (instance_id, current_task, data, timestamp) VALUES (?, ?, ?, ?)"
        )
        .bind(&checkpoint.instance_id)
        .bind(&checkpoint.current_task)
        .bind(&data_json)
        .bind(&timestamp_str)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to save checkpoint: {e}") })?;

        Ok(())
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        let result = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT instance_id, current_task, data, timestamp FROM workflow_checkpoints WHERE instance_id = ?"
        )
        .bind(instance_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to get checkpoint: {e}") })?;

        match result {
            Some((instance_id, current_task, data_json, timestamp_str)) => {
                let data = serde_json::from_str(&data_json).context(SerializationSnafu)?;
                let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                    .map_err(|e| Error::Database {
                        message: format!("Failed to parse timestamp: {e}"),
                    })?
                    .with_timezone(&chrono::Utc);

                Ok(Some(WorkflowCheckpoint {
                    instance_id,
                    current_task,
                    data,
                    timestamp,
                }))
            }
            None => Ok(None),
        }
    }

    async fn append_journal(&self, instance_id: &str, records: Vec<JournalRecord>) -> Result<()> {
        let mut transaction = self.pool.begin().await.map_err(|e| Error::Database {
            message: format!("Failed to begin a transaction: {e}"),
        })?;
        for (position, body) in records {
            sqlx::query(
                "INSERT INTO workflow_journal (instance_id, position, body) VALUES (?, ?, ?)",
            )
            .bind(instance_id)
            .bind(i64::try_from(position).unwrap_or(i64::MAX))
            .bind(serde_json::to_string(&body).context(SerializationSnafu)?)
            .execute(&mut *transaction)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to append a journal record: {e}"),
            })?;
        }
        transaction.commit().await.map_err(|e| Error::Database {
            message: format!("Failed to commit the journal: {e}"),
        })
    }

    async fn load_journal(&self, instance_id: &str) -> Result<Vec<JournalRecord>> {
        let rows = sqlx::query_as::<_, (i64, String)>(
            "SELECT position, body FROM workflow_journal WHERE instance_id = ? ORDER BY position ASC",
        )
        .bind(instance_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database {
            message: format!("Failed to load the journal: {e}"),
        })?;
        let mut records = Vec::with_capacity(rows.len());
        for (position, body) in rows {
            records.push((
                u64::try_from(position).unwrap_or(0),
                serde_json::from_str(&body).context(SerializationSnafu)?,
            ));
        }
        Ok(records)
    }

    async fn prune_journal(&self, instance_id: &str, before: u64) -> Result<()> {
        sqlx::query("DELETE FROM workflow_journal WHERE instance_id = ? AND position < ?")
            .bind(instance_id)
            .bind(i64::try_from(before).unwrap_or(i64::MAX))
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to prune the journal: {e}"),
            })?;
        Ok(())
    }

    async fn create_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<bool> {
        let result = sqlx::query("INSERT OR IGNORE INTO workflow_checkpoints (instance_id, current_task, data, timestamp) VALUES (?, ?, ?, ?)")
            .bind(&checkpoint.instance_id)
            .bind(&checkpoint.current_task)
            .bind(serde_json::to_string(&checkpoint.data).context(SerializationSnafu)?)
            .bind(checkpoint.timestamp.to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to create checkpoint: {e}"),
            })?;
        Ok(result.rows_affected() == 1)
    }

    async fn list_instances(&self) -> Result<Option<Vec<String>>> {
        let ids = sqlx::query_scalar::<_, String>("SELECT instance_id FROM workflow_checkpoints")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to list instances: {e}"),
            })?;
        Ok(Some(ids))
    }

    async fn delete_instance(&self, instance_id: &str) -> Result<()> {
        let mut transaction = self.pool.begin().await.map_err(|e| Error::Database {
            message: format!("Failed to begin a transaction: {e}"),
        })?;
        for table in [
            "workflow_checkpoints",
            "workflow_events",
            "workflow_journal",
            "workflow_leases",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE instance_id = ?"))
                .bind(instance_id)
                .execute(&mut *transaction)
                .await
                .map_err(|e| Error::Database {
                    message: format!("Failed to delete from {table}: {e}"),
                })?;
        }
        transaction.commit().await.map_err(|e| Error::Database {
            message: format!("Failed to commit the deletion: {e}"),
        })
    }

    fn supports_leases(&self) -> bool {
        true
    }

    async fn acquire_lease(
        &self,
        instance_id: &str,
        owner: &str,
        now: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<Lease> {
        // One statement: insert, or take over a lease that is ours or has expired. Then read what is there.
        sqlx::query(
            "INSERT INTO workflow_leases (instance_id, owner, expires_at) VALUES (?, ?, ?) \
             ON CONFLICT (instance_id) DO UPDATE SET owner = excluded.owner, expires_at = excluded.expires_at \
             WHERE workflow_leases.owner = excluded.owner OR workflow_leases.expires_at <= ?",
        )
        .bind(instance_id)
        .bind(owner)
        .bind(until.timestamp_millis())
        .bind(now.timestamp_millis())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database {
            message: format!("Failed to take a lease: {e}"),
        })?;
        let (owner, expires_at): (String, i64) =
            sqlx::query_as("SELECT owner, expires_at FROM workflow_leases WHERE instance_id = ?")
                .bind(instance_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| Error::Database {
                    message: format!("Failed to read a lease: {e}"),
                })?;
        Ok(Lease {
            owner,
            expires_at: chrono::DateTime::from_timestamp_millis(expires_at).unwrap_or_default(),
        })
    }

    async fn release_lease(&self, instance_id: &str, owner: &str) -> Result<()> {
        sqlx::query("DELETE FROM workflow_leases WHERE instance_id = ? AND owner = ?")
            .bind(instance_id)
            .bind(owner)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to release a lease: {e}"),
            })?;
        Ok(())
    }

    async fn post_message(&self, instance_id: &str, message: serde_json::Value) -> Result<()> {
        sqlx::query("INSERT INTO workflow_messages (instance_id, body) VALUES (?, ?)")
            .bind(instance_id)
            .bind(serde_json::to_string(&message).context(SerializationSnafu)?)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to post a message: {e}"),
            })?;
        Ok(())
    }

    async fn take_messages(&self, owner: &str) -> Result<Vec<(String, serde_json::Value)>> {
        let mut transaction = self.pool.begin().await.map_err(|e| Error::Database {
            message: format!("Failed to begin a transaction: {e}"),
        })?;
        let rows: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT id, instance_id, body FROM workflow_messages WHERE instance_id IN \
             (SELECT instance_id FROM workflow_leases WHERE owner = ?) ORDER BY id",
        )
        .bind(owner)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|e| Error::Database {
            message: format!("Failed to read messages: {e}"),
        })?;
        let mut taken = Vec::with_capacity(rows.len());
        for (id, instance_id, body) in rows {
            sqlx::query("DELETE FROM workflow_messages WHERE id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await
                .map_err(|e| Error::Database {
                    message: format!("Failed to remove a message: {e}"),
                })?;
            let body = serde_json::from_str(&body).context(SerializationSnafu)?;
            taken.push((instance_id, body));
        }
        transaction.commit().await.map_err(|e| Error::Database {
            message: format!("Failed to commit taking messages: {e}"),
        })?;
        Ok(taken)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::wildcard_enum_match_arm)]

    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_sqlite_persistence_events() {
        let persistence = SqlitePersistence::new(":memory:").await.unwrap();

        let instance_id = "test-instance-1";

        // Save events
        let event1 = WorkflowEvent::WorkflowStarted {
            instance_id: instance_id.to_string(),
            workflow_id: "workflow1".to_string(),
            timestamp: Utc::now(),
            initial_data: serde_json::json!({"input": "data"}),
        };

        let event2 = WorkflowEvent::TaskStarted {
            instance_id: instance_id.to_string(),
            task_name: "task1".to_string(),
            timestamp: Utc::now(),
        };

        persistence.save_event(event1).await.unwrap();
        persistence.save_event(event2).await.unwrap();

        // Retrieve events
        let events = persistence.get_events(instance_id).await.unwrap();
        assert_eq!(events.len(), 2);

        match events.first() {
            Some(WorkflowEvent::WorkflowStarted { workflow_id, .. }) => {
                assert_eq!(workflow_id, "workflow1");
            }
            _ => panic!("Expected WorkflowStarted event"),
        }

        match events.get(1) {
            Some(WorkflowEvent::TaskStarted { task_name, .. }) => {
                assert_eq!(task_name, "task1");
            }
            _ => panic!("Expected TaskStarted event"),
        }
    }

    #[tokio::test]
    async fn test_sqlite_persistence_checkpoint() {
        let persistence = SqlitePersistence::new(":memory:").await.unwrap();

        let checkpoint = WorkflowCheckpoint {
            instance_id: "test-instance-2".to_string(),
            current_task: "task2".to_string(),
            data: serde_json::json!({"state": "active"}),
            timestamp: Utc::now(),
        };

        persistence
            .save_checkpoint(checkpoint.clone())
            .await
            .unwrap();

        let retrieved = persistence
            .get_checkpoint("test-instance-2")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retrieved.instance_id, "test-instance-2");
        assert_eq!(retrieved.current_task, "task2");
        assert_eq!(retrieved.data, serde_json::json!({"state": "active"}));
    }

    #[tokio::test]
    async fn test_sqlite_persistence_checkpoint_upsert() {
        let persistence = SqlitePersistence::new(":memory:").await.unwrap();

        let checkpoint1 = WorkflowCheckpoint {
            instance_id: "test-instance-3".to_string(),
            current_task: "task1".to_string(),
            data: serde_json::json!({"step": 1}),
            timestamp: Utc::now(),
        };

        persistence.save_checkpoint(checkpoint1).await.unwrap();

        // Update checkpoint
        let checkpoint2 = WorkflowCheckpoint {
            instance_id: "test-instance-3".to_string(),
            current_task: "task2".to_string(),
            data: serde_json::json!({"step": 2}),
            timestamp: Utc::now(),
        };

        persistence.save_checkpoint(checkpoint2).await.unwrap();

        let retrieved = persistence
            .get_checkpoint("test-instance-3")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retrieved.current_task, "task2");
        assert_eq!(retrieved.data, serde_json::json!({"step": 2}));
    }

    #[tokio::test]
    async fn test_sqlite_persistence_event_ordering() {
        let persistence = SqlitePersistence::new(":memory:").await.unwrap();

        let instance_id = "test-instance-4";

        // Save multiple events
        for i in 0..5 {
            let event = WorkflowEvent::TaskCompleted {
                instance_id: instance_id.to_string(),
                task_name: format!("task{}", i),
                result: serde_json::json!({"step": i}),
                timestamp: Utc::now(),
                duration_ms: 100,
            };
            persistence.save_event(event).await.unwrap();
        }

        let events = persistence.get_events(instance_id).await.unwrap();
        assert_eq!(events.len(), 5);

        // Verify order
        for (i, event) in events.iter().enumerate() {
            match event {
                WorkflowEvent::TaskCompleted { task_name, .. } => {
                    assert_eq!(task_name, &format!("task{}", i));
                }
                _ => panic!("Expected TaskCompleted event"),
            }
        }
    }
}
