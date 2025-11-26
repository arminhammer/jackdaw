use crate::persistence::{Error, PersistenceProvider, Result};
use crate::workflow::{WorkflowCheckpoint, WorkflowEvent};
use async_trait::async_trait;
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Debug)]
pub struct PostgresPersistence {
    pool: PgPool,
}

impl PostgresPersistence {
    /// Create a new PostgreSQL persistence provider
    ///
    /// # Arguments
    /// * `database_url` - PostgreSQL connection string (e.g., "postgresql://user:pass@localhost/db")
    ///
    /// # Example
    /// ```no_run
    /// # use jackdaw::providers::persistence::PostgresPersistence;
    /// # async fn example() -> anyhow::Result<()> {
    /// let persistence = PostgresPersistence::new("postgresql://user:password@localhost/mydb").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to connect to PostgreSQL: {}", e),
            })?;

        // Initialize schema - execute statements individually since PostgreSQL
        // prepared statements don't support multiple statements
        let schema_sql = include_str!("./sql/persistence_postgres.sql");
        for statement in schema_sql.split(';').filter(|s| !s.trim().is_empty()) {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|e| Error::Database {
                    message: format!("Failed to execute schema statement: {}", e),
                })?;
        }

        Ok(Self { pool })
    }

    /// Create a new PostgreSQL persistence provider with custom pool options
    pub async fn with_pool(pool: PgPool) -> Result<Self> {
        // Initialize schema - execute statements individually since PostgreSQL
        // prepared statements don't support multiple statements
        let schema_sql = include_str!("./sql/persistence_postgres.sql");
        for statement in schema_sql.split(';').filter(|s| !s.trim().is_empty()) {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|e| Error::Database {
                    message: format!("Failed to execute schema statement: {}", e),
                })?;
        }

        Ok(Self { pool })
    }

    /// Get the event type name for a WorkflowEvent
    fn get_event_type(event: &WorkflowEvent) -> &'static str {
        match event {
            WorkflowEvent::WorkflowStarted { .. } => "WorkflowStarted",
            WorkflowEvent::TaskEntered { .. } => "TaskEntered",
            WorkflowEvent::TaskStarted { .. } => "TaskStarted",
            WorkflowEvent::TaskCompleted { .. } => "TaskCompleted",
            WorkflowEvent::WorkflowCompleted { .. } => "WorkflowCompleted",
            WorkflowEvent::WorkflowFailed { .. } => "WorkflowFailed",
        }
    }
}

#[async_trait]
impl PersistenceProvider for PostgresPersistence {
    async fn save_event(&self, event: WorkflowEvent) -> Result<()> {
        let instance_id = event.instance_id().to_string();
        let event_type = Self::get_event_type(&event);
        let event_data =
            serde_json::to_value(&event).map_err(|e| Error::Serialization { source: e })?;
        let timestamp = chrono::Utc::now();

        // Get the next sequence number for this instance
        let sequence_number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence_number), -1) + 1 FROM workflow_events WHERE instance_id = $1"
        )
        .bind(&instance_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to get sequence number: {}", e) })?;

        sqlx::query(
            "INSERT INTO workflow_events (instance_id, event_type, event_data, timestamp, sequence_number) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(&instance_id)
        .bind(event_type)
        .bind(&event_data)
        .bind(&timestamp)
        .bind(sequence_number)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to save event: {}", e) })?;

        Ok(())
    }

    async fn get_events(&self, instance_id: &str) -> Result<Vec<WorkflowEvent>> {
        let rows = sqlx::query_as::<_, (serde_json::Value,)>(
            "SELECT event_data FROM workflow_events WHERE instance_id = $1 ORDER BY sequence_number ASC"
        )
        .bind(instance_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to get events: {}", e) })?;

        let mut events = Vec::new();
        for (event_data,) in rows {
            let event: WorkflowEvent = serde_json::from_value(event_data)
                .map_err(|e| Error::Serialization { source: e })?;
            events.push(event);
        }

        Ok(events)
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> Result<()> {
        let data_json = serde_json::to_value(&checkpoint.data)
            .map_err(|e| Error::Serialization { source: e })?;

        sqlx::query(
            r#"
            INSERT INTO workflow_checkpoints (instance_id, current_task, data, timestamp)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (instance_id)
            DO UPDATE SET
                current_task = EXCLUDED.current_task,
                data = EXCLUDED.data,
                timestamp = EXCLUDED.timestamp
            "#,
        )
        .bind(&checkpoint.instance_id)
        .bind(&checkpoint.current_task)
        .bind(&data_json)
        .bind(&checkpoint.timestamp)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database {
            message: format!("Failed to save checkpoint: {}", e),
        })?;

        Ok(())
    }

    async fn get_checkpoint(&self, instance_id: &str) -> Result<Option<WorkflowCheckpoint>> {
        let result = sqlx::query_as::<_, (String, String, serde_json::Value, chrono::DateTime<chrono::Utc>)>(
            "SELECT instance_id, current_task, data, timestamp FROM workflow_checkpoints WHERE instance_id = $1"
        )
        .bind(instance_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to get checkpoint: {}", e) })?;

        match result {
            Some((instance_id, current_task, data, timestamp)) => Ok(Some(WorkflowCheckpoint {
                instance_id,
                current_task,
                data,
                timestamp,
            })),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use testcontainers::{GenericImage, ImageExt, runners::AsyncRunner};

    async fn setup_postgres_container() -> (testcontainers::ContainerAsync<GenericImage>, String) {
        use testcontainers::core::ContainerPort;

        let postgres_image = GenericImage::new("postgres", "16-alpine")
            .with_exposed_port(ContainerPort::Tcp(5432))
            .with_env_var("POSTGRES_DB", "test_db")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres");

        let container = postgres_image
            .start()
            .await
            .expect("Failed to start postgres container");
        let port = container
            .get_host_port_ipv4(ContainerPort::Tcp(5432))
            .await
            .expect("Failed to get port");
        let database_url = format!("postgresql://postgres:postgres@localhost:{}/test_db", port);

        // Wait for PostgreSQL to be fully ready and accept connections
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        (container, database_url)
    }

    #[tokio::test]
    async fn test_postgres_persistence_events() {
        let (_container, database_url) = setup_postgres_container().await;
        let persistence = PostgresPersistence::new(&database_url).await.unwrap();

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

        match &events[0] {
            WorkflowEvent::WorkflowStarted { workflow_id, .. } => {
                assert_eq!(workflow_id, "workflow1");
            }
            _ => panic!("Expected WorkflowStarted event"),
        }

        match &events[1] {
            WorkflowEvent::TaskStarted { task_name, .. } => {
                assert_eq!(task_name, "task1");
            }
            _ => panic!("Expected TaskStarted event"),
        }
    }

    #[tokio::test]
    async fn test_postgres_persistence_checkpoint() {
        let (_container, database_url) = setup_postgres_container().await;
        let persistence = PostgresPersistence::new(&database_url).await.unwrap();

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
    async fn test_postgres_persistence_checkpoint_upsert() {
        let (_container, database_url) = setup_postgres_container().await;
        let persistence = PostgresPersistence::new(&database_url).await.unwrap();

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
    async fn test_postgres_persistence_event_ordering() {
        let (_container, database_url) = setup_postgres_container().await;
        let persistence = PostgresPersistence::new(&database_url).await.unwrap();

        let instance_id = "test-instance-4";

        // Save multiple events
        for i in 0..5 {
            let event = WorkflowEvent::TaskCompleted {
                instance_id: instance_id.to_string(),
                task_name: format!("task{}", i),
                result: serde_json::json!({"step": i}),
                timestamp: Utc::now(),
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
