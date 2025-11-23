use crate::cache::{CacheEntry, CacheProvider, Error, Result};
use async_trait::async_trait;
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Debug)]
pub struct PostgresCache {
    pool: PgPool,
}

impl PostgresCache {
    /// Create a new PostgreSQL cache provider
    ///
    /// # Arguments
    /// * `database_url` - PostgreSQL connection string (e.g., "postgresql://user:pass@localhost/db")
    ///
    /// # Example
    /// ```no_run
    /// # use qyvx::providers::cache::PostgresCache;
    /// # async fn example() -> anyhow::Result<()> {
    /// let cache = PostgresCache::new("postgresql://user:password@localhost/mydb").await?;
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
        let schema_sql = include_str!("./sql/cache_postgres.sql");
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

    /// Create a new PostgreSQL cache with custom pool options
    pub async fn with_pool(pool: PgPool) -> Result<Self> {
        // Initialize schema - execute statements individually since PostgreSQL
        // prepared statements don't support multiple statements
        let schema_sql = include_str!("./sql/cache_postgres.sql");
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
}

#[async_trait]
impl CacheProvider for PostgresCache {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>> {
        let result =
            sqlx::query_as::<
                _,
                (
                    String,
                    serde_json::Value,
                    serde_json::Value,
                    chrono::DateTime<chrono::Utc>,
                ),
            >("SELECT key, inputs, output, timestamp FROM cache_entries WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to get cache entry: {}", e),
            })?;

        match result {
            Some((key, inputs, output, timestamp)) => Ok(Some(CacheEntry {
                key,
                inputs,
                output,
                timestamp,
            })),
            None => Ok(None),
        }
    }

    async fn set(&self, entry: CacheEntry) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO cache_entries (key, inputs, output, timestamp)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (key)
            DO UPDATE SET
                inputs = EXCLUDED.inputs,
                output = EXCLUDED.output,
                timestamp = EXCLUDED.timestamp
            "#,
        )
        .bind(&entry.key)
        .bind(&entry.inputs)
        .bind(&entry.output)
        .bind(&entry.timestamp)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database {
            message: format!("Failed to set cache entry: {}", e),
        })?;

        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM cache_entries WHERE key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to invalidate cache entry: {}", e),
            })?;

        Ok(())
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
    async fn test_postgres_cache_basic_operations() {
        let (_container, database_url) = setup_postgres_container().await;
        let cache = PostgresCache::new(&database_url).await.unwrap();

        // Test set and get
        let entry = CacheEntry {
            key: "test_key".to_string(),
            inputs: serde_json::json!({"param": "value"}),
            output: serde_json::json!({"result": "success"}),
            timestamp: Utc::now(),
        };

        cache.set(entry.clone()).await.unwrap();

        let retrieved = cache.get("test_key").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.key, "test_key");
        assert_eq!(retrieved.inputs, serde_json::json!({"param": "value"}));
        assert_eq!(retrieved.output, serde_json::json!({"result": "success"}));

        // Test invalidate
        cache.invalidate("test_key").await.unwrap();
        let retrieved = cache.get("test_key").await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_postgres_cache_upsert() {
        let (_container, database_url) = setup_postgres_container().await;
        let cache = PostgresCache::new(&database_url).await.unwrap();

        let entry1 = CacheEntry {
            key: "key1".to_string(),
            inputs: serde_json::json!({"v": 1}),
            output: serde_json::json!({"r": 1}),
            timestamp: Utc::now(),
        };

        cache.set(entry1).await.unwrap();

        // Overwrite with new value
        let entry2 = CacheEntry {
            key: "key1".to_string(),
            inputs: serde_json::json!({"v": 2}),
            output: serde_json::json!({"r": 2}),
            timestamp: Utc::now(),
        };

        cache.set(entry2).await.unwrap();

        let retrieved = cache.get("key1").await.unwrap().unwrap();
        assert_eq!(retrieved.output, serde_json::json!({"r": 2}));
    }

    #[tokio::test]
    async fn test_postgres_cache_get_nonexistent() {
        let (_container, database_url) = setup_postgres_container().await;
        let cache = PostgresCache::new(&database_url).await.unwrap();
        let result = cache.get("nonexistent").await.unwrap();
        assert!(result.is_none());
    }
}
