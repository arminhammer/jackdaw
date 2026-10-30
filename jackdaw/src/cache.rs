//! Caching task results by input: the [`CacheProvider`] trait. The built-in caches are in
//! [`providers::cache`](crate::providers::cache).
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use snafu::prelude::*;

/// Why a cache operation failed.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
#[non_exhaustive]
pub enum Error {
    /// The cache refused the operation.
    #[snafu(display("Cache error: {message}"))]
    Cache {
        /// What went wrong.
        message: String,
    },

    /// An entry could not be (de)serialized.
    #[snafu(display("Serialization error: {source}"))]
    Serialization {
        /// The underlying error.
        source: serde_json::Error,
    },

    /// The database behind the cache failed.
    #[snafu(display("Database error: {message}"))]
    Database {
        /// What went wrong.
        message: String,
    },
}

/// The result of a cache operation.
pub type Result<T> = std::result::Result<T, Error>;

/// A cached task result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The key: a hash of the task's name and input.
    pub key: String,
    /// The input the result was computed from.
    pub inputs: serde_json::Value,
    /// The task's output.
    pub output: serde_json::Value,
    /// When the entry was written.
    pub timestamp: DateTime<Utc>,
}

/// Where cached task results live. Tasks opt in with `#[task(cached)]` or `.cached()` on a step; equal input then
/// returns the stored output instead of running the task again.
#[async_trait]
#[allow(dead_code)]
pub trait CacheProvider: Send + Sync + std::fmt::Debug {
    /// The entry under `key`, if any.
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>>;
    /// Stores `entry`, replacing an entry with the same key.
    async fn set(&self, entry: CacheEntry) -> Result<()>;
    /// Removes the entry under `key`.
    async fn invalidate(&self, key: &str) -> Result<()>;
}

// Helper to filter out internal descriptor fields from cache key computation
// Recursively removes fields starting with "__" from objects
fn filter_internal_fields(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(obj) => {
            let filtered: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .filter(|(key, _)| !key.starts_with("__"))
                .map(|(k, v)| (k.clone(), filter_internal_fields(v)))
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(filter_internal_fields).collect())
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => value.clone(),
    }
}

// Helper to generate deterministic cache keys
// Note: Filters out internal descriptor fields (__workflow, __runtime, __task)
// so they don't affect caching
// Uses SHA-256 for deterministic hashing across runs
/// The cache key of a call: a SHA-256 hash of the task name and its input, ignoring fields that start with `__`.
#[must_use]
pub fn compute_cache_key(task_name: &str, inputs: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};

    let filtered_inputs = filter_internal_fields(inputs);

    // Normalize the JSON by sorting keys for deterministic serialization
    let normalized = normalize_json(&filtered_inputs);
    let inputs_json = serde_json::to_string(&normalized).unwrap_or_default();

    let mut hasher = Sha256::new();
    hasher.update(format!("{task_name}:{inputs_json}"));
    let result = hasher.finalize();

    format!("{}:{:x}", task_name, result)
}

/// Normalize JSON by recursively sorting object keys for deterministic serialization
fn normalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: Vec<_> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let normalized_map: serde_json::Map<String, serde_json::Value> = sorted
                .into_iter()
                .map(|(k, v)| (k.clone(), normalize_json(v)))
                .collect();
            serde_json::Value::Object(normalized_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(normalize_json).collect())
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => value.clone(),
    }
}

// pub const CACHE_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("cache");
