use async_trait::async_trait;
use std::sync::Arc;
use crate::cache::{CacheProvider, CacheEntry, Result, Error};

const CACHE_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("cache");

#[derive(Debug)]
pub struct RedbCache {
    db: Arc<redb::Database>,
}

impl RedbCache {
    pub fn new(db: Arc<redb::Database>) -> Result<Self> {
        let write_txn = db.begin_write()
            .map_err(|e| Error::Database { message: format!("Failed to begin write transaction: {}", e) })?;
        {
            write_txn.open_table(CACHE_TABLE)
                .map_err(|e| Error::Database { message: format!("Failed to open cache table: {}", e) })?;
        }
        write_txn.commit()
            .map_err(|e| Error::Database { message: format!("Failed to commit transaction: {}", e) })?;
        Ok(Self { db })
    }
}

#[async_trait]
impl CacheProvider for RedbCache {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>> {
        let db = self.db.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<CacheEntry>> {
            let read_txn = db.begin_read()
                .map_err(|e| Error::Database { message: format!("Failed to begin read transaction: {}", e) })?;
            let table = read_txn.open_table(CACHE_TABLE)
                .map_err(|e| Error::Database { message: format!("Failed to open cache table: {}", e) })?;
            if let Some(value) = table.get(key.as_str())
                .map_err(|e| Error::Database { message: format!("Failed to get value: {}", e) })? {
                let entry: CacheEntry = serde_json::from_slice(value.value())
                    .map_err(|e| Error::Serialization { source: e })?;
                Ok(Some(entry))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }

    async fn set(&self, entry: CacheEntry) -> Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write()
                .map_err(|e| Error::Database { message: format!("Failed to begin write transaction: {}", e) })?;
            {
                let mut table = write_txn.open_table(CACHE_TABLE)
                    .map_err(|e| Error::Database { message: format!("Failed to open cache table: {}", e) })?;
                let value = serde_json::to_vec(&entry)
                    .map_err(|e| Error::Serialization { source: e })?;
                table.insert(entry.key.as_str(), value.as_slice())
                    .map_err(|e| Error::Database { message: format!("Failed to insert value: {}", e) })?;
            }
            write_txn.commit()
                .map_err(|e| Error::Database { message: format!("Failed to commit transaction: {}", e) })?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }

    async fn invalidate(&self, key: &str) -> Result<()> {
        let db = self.db.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write()
                .map_err(|e| Error::Database { message: format!("Failed to begin write transaction: {}", e) })?;
            {
                let mut table = write_txn.open_table(CACHE_TABLE)
                    .map_err(|e| Error::Database { message: format!("Failed to open cache table: {}", e) })?;
                table.remove(key.as_str())
                    .map_err(|e| Error::Database { message: format!("Failed to remove value: {}", e) })?;
            }
            write_txn.commit()
                .map_err(|e| Error::Database { message: format!("Failed to commit transaction: {}", e) })?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Database { message: format!("Task join error: {}", e) })?
    }
}
