//! Builder for configuring and creating a [`DurableEngine`](crate::durableengine::DurableEngine)
//!
//! The [`DurableEngineBuilder`] provides a fluent API for configuring engine components.

use crate::{
    cache::CacheProvider,
    durableengine::{DurableEngine, Result},
    persistence::PersistenceProvider,
    providers::{cache::mem::InMemoryCache, persistence::InMemoryPersistence},
};
use std::sync::Arc;

/// Builder for creating a [`DurableEngine`](crate::durableengine::DurableEngine)
///
/// This builder provides a fluent API for configuring the engine with custom
/// persistence and cache providers. If not specified, the builder defaults to
/// in-memory providers suitable for testing and ephemeral workflows.
///
/// # Examples
///
/// ## Default configuration (in-memory)
/// ```
/// use jackdaw::DurableEngineBuilder;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let engine = DurableEngineBuilder::new().build()?;
/// # Ok(())
/// # }
/// ```
///
/// ## Custom persistence and cache
/// ```
/// use jackdaw::DurableEngineBuilder;
/// use jackdaw::providers::persistence::RedbPersistence;
/// use jackdaw::providers::cache::RedbCache;
/// use std::sync::Arc;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let persistence = Arc::new(RedbPersistence::new("./workflow.db")?);
/// let cache = Arc::new(RedbCache::new(persistence.db.clone())?);
///
/// let engine = DurableEngineBuilder::new()
///     .with_persistence(persistence)
///     .with_cache(cache)
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct DurableEngineBuilder {
    persistence: Option<Arc<dyn PersistenceProvider>>,
    cache: Option<Arc<dyn CacheProvider>>,
    event_buffer_size: usize,
}

impl DurableEngineBuilder {
    /// Create a new builder with default settings
    ///
    /// By default:
    /// - Uses in-memory persistence (not persisted across restarts)
    /// - Uses in-memory cache (not persisted across restarts)
    /// - Event buffer size of 1000
    #[must_use]
    pub fn new() -> Self {
        Self {
            persistence: None,
            cache: None,
            event_buffer_size: 1000,
        }
    }

    /// Set the persistence provider
    ///
    /// If not set, an in-memory persistence provider will be used by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use jackdaw::DurableEngineBuilder;
    /// use jackdaw::providers::persistence::RedbPersistence;
    /// use std::sync::Arc;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let persistence = Arc::new(RedbPersistence::new("./workflow.db")?);
    ///
    /// let engine = DurableEngineBuilder::new()
    ///     .with_persistence(persistence)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_persistence(mut self, persistence: Arc<dyn PersistenceProvider>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Set the cache provider
    ///
    /// If not set, an in-memory cache provider will be used by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use jackdaw::DurableEngineBuilder;
    /// use jackdaw::providers::cache::mem::InMemoryCache;
    /// use std::sync::Arc;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let cache = Arc::new(InMemoryCache::new());
    ///
    /// let engine = DurableEngineBuilder::new()
    ///     .with_cache(cache)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<dyn CacheProvider>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set the event buffer size for streaming execution
    ///
    /// This controls how many events can be buffered before backpressure is applied.
    /// Default is 1000.
    ///
    /// A larger buffer size allows more events to be queued if the consumer is slow,
    /// but uses more memory. A smaller buffer size applies backpressure sooner.
    ///
    /// # Examples
    ///
    /// ```
    /// use jackdaw::DurableEngineBuilder;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let engine = DurableEngineBuilder::new()
    ///     .with_event_buffer_size(5000)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_event_buffer_size(mut self, size: usize) -> Self {
        self.event_buffer_size = size;
        self
    }

    /// Build the engine
    ///
    /// This creates the [`DurableEngine`](crate::durableengine::DurableEngine) with
    /// the configured settings. If persistence or cache providers were not specified,
    /// in-memory defaults will be used.
    ///
    /// # Errors
    /// Returns an error if the engine cannot be initialized.
    ///
    /// # Examples
    ///
    /// ```
    /// use jackdaw::DurableEngineBuilder;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let engine = DurableEngineBuilder::new().build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn build(self) -> Result<DurableEngine> {
        // Use in-memory providers as defaults
        let persistence = self
            .persistence
            .unwrap_or_else(|| Arc::new(InMemoryPersistence::new()));

        let cache = self.cache.unwrap_or_else(|| Arc::new(InMemoryCache::new()));

        DurableEngine::new_with_config(persistence, cache, self.event_buffer_size)
    }
}

impl Default for DurableEngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default() {
        let engine = DurableEngineBuilder::new().build();
        assert!(engine.is_ok());
    }

    #[test]
    fn test_builder_with_custom_buffer_size() {
        let engine = DurableEngineBuilder::new()
            .with_event_buffer_size(5000)
            .build();
        assert!(engine.is_ok());
    }

    #[test]
    fn test_builder_with_in_memory_providers() {
        let persistence = Arc::new(InMemoryPersistence::new());
        let cache = Arc::new(InMemoryCache::new());

        let engine = DurableEngineBuilder::new()
            .with_persistence(persistence)
            .with_cache(cache)
            .build();

        assert!(engine.is_ok());
    }
}
