//! Durable stores: where an engine keeps journals, history and cached task results.
//!
//! An engine built without a store keeps everything in memory. [`Redb`] is a single file and needs no server;
//! [`Sqlite`] and [`Postgres`] keep everything in one database, and several processes can share a Postgres store.
//! Anything that implements [`Store`] can be passed to [`EngineBuilder::store`](super::EngineBuilder::store).
use super::error::Error;
use crate::cache::CacheProvider;
use crate::persistence::PersistenceProvider;
use crate::providers::cache::{PostgresCache, RedbCache, SqliteCache};
use crate::providers::persistence::{PostgresPersistence, RedbPersistence, SqlitePersistence};
use std::path::Path;
use std::sync::Arc;

/// A durable store: persistence for journals and history, and a cache for task results.
///
/// Implement it to plug in your own storage; the built-in stores are [`Redb`], [`Sqlite`] and [`Postgres`].
pub trait Store: Send + Sync + 'static {
    /// Where journals, checkpoints and history go.
    fn persistence(&self) -> Arc<dyn PersistenceProvider>;
    /// Where cached task results go.
    fn cache(&self) -> Arc<dyn CacheProvider>;
}

/// Persistence and cache in one `redb` file. Cloning shares the same open file.
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
}

impl Store for Redb {
    fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}

/// Persistence and cache in one SQLite database. Cloning shares the same connection pool.
#[derive(Clone)]
pub struct Sqlite {
    persistence: Arc<SqlitePersistence>,
    cache: Arc<SqliteCache>,
}

impl std::fmt::Debug for Sqlite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sqlite").finish_non_exhaustive()
    }
}

impl Sqlite {
    /// Opens (creating if needed) the database file at `path`.
    ///
    /// # Errors
    /// If the database cannot be opened or its schema cannot be created.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let text = path.to_str().ok_or_else(|| {
            Error::invalid(format!("the path {} is not valid UTF-8", path.display()))
        })?;
        Self::connect(&format!("sqlite:{text}?mode=rwc")).await
    }

    /// Connects with a SQLite connection string, for example `"sqlite::memory:"`.
    ///
    /// # Errors
    /// If the database cannot be opened or its schema cannot be created.
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .map_err(|e| Error::Engine(format!("cannot open SQLite at {url}: {e}")))?;
        Ok(Self {
            persistence: Arc::new(SqlitePersistence::with_pool(pool.clone()).await?),
            cache: Arc::new(SqliteCache::with_pool(pool).await?),
        })
    }
}

impl Store for Sqlite {
    fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}

/// Persistence and cache in one PostgreSQL database. Cloning shares the same connection pool.
#[derive(Clone)]
pub struct Postgres {
    persistence: Arc<PostgresPersistence>,
    cache: Arc<PostgresCache>,
}

impl std::fmt::Debug for Postgres {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Postgres").finish_non_exhaustive()
    }
}

impl Postgres {
    /// Connects to `url` (`postgres://user:password@host/database`) and creates the schema if needed.
    ///
    /// # Errors
    /// If the database cannot be reached or its schema cannot be created.
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .map_err(|e| Error::Engine(format!("cannot connect to Postgres: {e}")))?;
        Ok(Self {
            persistence: Arc::new(PostgresPersistence::with_pool(pool.clone()).await?),
            cache: Arc::new(PostgresCache::with_pool(pool).await?),
        })
    }
}

impl Store for Postgres {
    fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}
