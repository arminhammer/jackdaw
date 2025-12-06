use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("Cache error: {message}"))]
    Cache { message: String },

    #[snafu(display("Serialization error: {source}"))]
    Serialization { source: serde_json::Error },

    #[snafu(display("Database error: {message}"))]
    Database { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub inputs: serde_json::Value,
    pub output: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

/// Pluggable cache provider for idempotent task execution
#[async_trait]
#[allow(dead_code)]
pub trait CacheProvider: Send + Sync + std::fmt::Debug {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>>;
    async fn set(&self, entry: CacheEntry) -> Result<()>;
    async fn invalidate(&self, key: &str) -> Result<()>;
}

// Helper to filter out internal descriptor fields from cache key computation
fn filter_internal_fields(value: &serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object() {
        let filtered: serde_json::Map<String, serde_json::Value> = obj
            .iter()
            .filter(|(key, _)| !key.starts_with("__"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::Value::Object(filtered)
    } else {
        value.clone()
    }
}

// Helper to generate deterministic cache keys
// Note: Filters out internal descriptor fields (__workflow, __runtime, __task)
// so they don't affect caching
#[must_use]
pub fn compute_cache_key(task_name: &str, inputs: &serde_json::Value) -> String {
    let filtered_inputs = filter_internal_fields(inputs);
    let inputs_json = serde_json::to_string(&filtered_inputs).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    format!("{task_name}:{inputs_json}").hash(&mut hasher);
    format!("{}:{:x}", task_name, hasher.finish())
}

// pub const CACHE_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("cache");
