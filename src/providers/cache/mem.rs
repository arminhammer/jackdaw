use crate::cache::{CacheEntry, CacheProvider, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct InMemoryCache {
    store: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl InMemoryCache {
    #[allow(dead_code)]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CacheProvider for InMemoryCache {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>> {
        let store = self.store.lock().unwrap();
        Ok(store.get(key).cloned())
    }

    async fn set(&self, entry: CacheEntry) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        store.insert(entry.key.clone(), entry);
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        store.remove(key);
        Ok(())
    }
}
