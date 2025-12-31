use crate::cache::{CacheEntry, CacheProvider, Error, Result, SerializationSnafu};
use async_trait::async_trait;
use snafu::prelude::*;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

#[derive(Debug)]
#[allow(dead_code)]
pub struct SqliteCache {
    pool: SqlitePool,
}

#[allow(dead_code)]
impl SqliteCache {
    /// Create a new ``SQLite`` cache provider
    ///
    /// # Arguments
    /// * `database_url` - ``SQLite`` connection string (e.g., `cache.db` or `:memory:`)
    ///
    /// # Errors
    /// Returns an error if the database connection fails or if the schema initialization fails.
    ///
    /// # Example
    /// ```no_run
    /// # use jackdaw::providers::cache::SqliteCache;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let cache = SqliteCache::new(":memory:").await?;
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
        sqlx::query(include_str!("./sql/cache_sqlite.sql"))
            .execute(&pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to execute schema: {e}"),
            })?;

        Ok(Self { pool })
    }

    /// Create a new ``SQLite`` cache with custom pool options
    ///
    /// # Errors
    /// Returns an error if the schema initialization fails.
    pub async fn with_pool(pool: SqlitePool) -> Result<Self> {
        // Initialize schema
        sqlx::query(include_str!("./sql/cache_sqlite.sql"))
            .execute(&pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to execute schema: {e}"),
            })?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl CacheProvider for SqliteCache {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>> {
        let result = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT key, inputs, output, timestamp FROM cache_entries WHERE key = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Database {
            message: format!("Failed to get cache entry: {e}"),
        })?;

        match result {
            Some((key, inputs_json, output_json, timestamp_str)) => {
                let inputs = serde_json::from_str(&inputs_json).context(SerializationSnafu)?;
                let output = serde_json::from_str(&output_json).context(SerializationSnafu)?;
                let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                    .map_err(|e| Error::Database {
                        message: format!("Failed to parse timestamp: {e}"),
                    })?
                    .with_timezone(&chrono::Utc);

                Ok(Some(CacheEntry {
                    key,
                    inputs,
                    output,
                    timestamp,
                }))
            }
            None => Ok(None),
        }
    }

    async fn set(&self, entry: CacheEntry) -> Result<()> {
        let inputs_json = serde_json::to_string(&entry.inputs).context(SerializationSnafu)?;
        let output_json = serde_json::to_string(&entry.output).context(SerializationSnafu)?;
        let timestamp_str = entry.timestamp.to_rfc3339();

        sqlx::query(
            "INSERT OR REPLACE INTO cache_entries (key, inputs, output, timestamp) VALUES (?, ?, ?, ?)"
        )
        .bind(&entry.key)
        .bind(&inputs_json)
        .bind(&output_json)
        .bind(&timestamp_str)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Database { message: format!("Failed to set cache entry: {e}") })?;

        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM cache_entries WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Database {
                message: format!("Failed to invalidate cache entry: {e}"),
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]

    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_sqlite_cache_basic_operations() {
        let cache = SqliteCache::new(":memory:").await.unwrap();

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
    async fn test_sqlite_cache_upsert() {
        let cache = SqliteCache::new(":memory:").await.unwrap();

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
    async fn test_sqlite_cache_get_nonexistent() {
        let cache = SqliteCache::new(":memory:").await.unwrap();
        let result = cache.get("nonexistent").await.unwrap();
        assert!(result.is_none());
    }
}
