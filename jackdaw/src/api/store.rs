//! Durable stores: where an engine keeps journals, history and cached task results.
//!
//! An engine built without a store keeps everything in memory. [`Redb`] is a single file and needs no server;
//! [`Sqlite`] and [`Postgres`] keep everything in one database, and several processes can share a Postgres store.
//! Anything that implements [`Store`] can be passed to [`EngineBuilder::store`](super::EngineBuilder::store).
#[cfg(any(feature = "redb", feature = "sqlite", feature = "postgres"))]
use super::error::Error;
use crate::cache::CacheProvider;
use crate::persistence::PersistenceProvider;
#[cfg(feature = "postgres")]
use crate::providers::{cache::PostgresCache, persistence::PostgresPersistence};
#[cfg(feature = "redb")]
use crate::providers::{cache::RedbCache, persistence::RedbPersistence};
#[cfg(feature = "sqlite")]
use crate::providers::{cache::SqliteCache, persistence::SqlitePersistence};
#[cfg(any(feature = "redb", feature = "sqlite"))]
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
#[cfg(feature = "redb")]
#[cfg_attr(docsrs, doc(cfg(feature = "redb")))]
#[derive(Clone)]
pub struct Redb {
    persistence: Arc<RedbPersistence>,
    cache: Arc<RedbCache>,
}

#[cfg(feature = "redb")]
impl std::fmt::Debug for Redb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redb").finish_non_exhaustive()
    }
}

#[cfg(feature = "redb")]
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

#[cfg(feature = "redb")]
impl Store for Redb {
    fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}

/// Persistence and cache in one SQLite database. Cloning shares the same connection pool.
#[cfg(feature = "sqlite")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite")))]
#[derive(Clone)]
pub struct Sqlite {
    persistence: Arc<SqlitePersistence>,
    cache: Arc<SqliteCache>,
}

#[cfg(feature = "sqlite")]
impl std::fmt::Debug for Sqlite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sqlite").finish_non_exhaustive()
    }
}

#[cfg(feature = "sqlite")]
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

#[cfg(feature = "sqlite")]
impl Store for Sqlite {
    fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}

/// Persistence and cache in one PostgreSQL database. Cloning shares the same connection pool.
#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
#[derive(Clone)]
pub struct Postgres {
    persistence: Arc<PostgresPersistence>,
    cache: Arc<PostgresCache>,
}

#[cfg(feature = "postgres")]
impl std::fmt::Debug for Postgres {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Postgres").finish_non_exhaustive()
    }
}

#[cfg(feature = "postgres")]
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

#[cfg(feature = "postgres")]
impl Store for Postgres {
    fn persistence(&self) -> Arc<dyn PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn PersistenceProvider>
    }

    fn cache(&self) -> Arc<dyn CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn CacheProvider>
    }
}
