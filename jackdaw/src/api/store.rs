//! A durable store on a single `redb` file.
use super::error::Error;
use crate::cache::CacheProvider;
use crate::persistence::PersistenceProvider;
use crate::providers::cache::RedbCache;
use crate::providers::persistence::RedbPersistence;
use std::path::Path;
use std::sync::Arc;

/// Persistence and cache in one file. Cloning shares the same open file.
#[derive(Clone)]
pub struct Redb {
    persistence: Arc<RedbPersistence>,
    cache: Arc<RedbCache>,
}

impl std::fmt::Debug for Redb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redb").finish_non_exhaustive()
    }
}

impl Redb {
    /// Opens (creating if needed) the database at `path`.
    ///
    /// # Errors
    /// If the file cannot be opened, for example because another engine in this process still has it open.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let text = path.to_str().ok_or_else(|| {
            Error::invalid(format!("the path {} is not valid UTF-8", path.display()))
        })?;
        let persistence = Arc::new(RedbPersistence::new(text)?);
        let cache = Arc::new(RedbCache::new(Arc::clone(&persistence.db))?);
        Ok(Self { persistence, cache })
    }

    pub(crate) fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    pub(crate) fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}
