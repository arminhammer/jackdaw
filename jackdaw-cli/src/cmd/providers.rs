//! The persistence and cache providers the `--persistence-provider` and `--cache-provider` flags choose.

use clap::Args;
use jackdaw::providers::cache::{PostgresCache, RedbCache, SqliteCache, mem::InMemoryCache};
use jackdaw::providers::persistence::{
    InMemoryPersistence, PostgresPersistence, RedbPersistence, SqlitePersistence,
};
use jackdaw::{CacheProvider, PersistenceProvider};
use std::path::PathBuf;
use std::sync::Arc;

use super::run::{Error, Result};
use crate::config::JackdawConfig;

#[derive(Args, Debug)]
pub struct ProviderArgs {
    /// Persistence provider to use (memory, redb, sqlite, postgres)
    #[arg(long, value_name = "PERSISTENCE_PROVIDER", default_value = "memory")]
    pub persistence_provider: String,

    /// Cache provider to use (memory, redb, sqlite, postgres)
    #[arg(long, value_name = "CACHE_PROVIDER", default_value = "memory")]
    pub cache_provider: String,

    /// SQLite database URL (e.g., 'workflow.db' or ':memory:')
    #[arg(long, value_name = "SQLITE_DB_URL", env = "SQLITE_DB_URL")]
    pub sqlite_db_url: Option<String>,

    /// PostgreSQL database name
    #[arg(long, value_name = "POSTGRES_DB_NAME", env = "POSTGRES_DB_NAME")]
    pub postgres_db_name: Option<String>,

    /// PostgreSQL user
    #[arg(long, value_name = "POSTGRES_USER", env = "POSTGRES_USER")]
    pub postgres_user: Option<String>,

    /// PostgreSQL password
    #[arg(long, value_name = "POSTGRES_PASSWORD", env = "POSTGRES_PASSWORD")]
    pub postgres_password: Option<String>,

    /// PostgreSQL hostname
    #[arg(long, value_name = "POSTGRES_HOSTNAME", env = "POSTGRES_HOSTNAME")]
    pub postgres_hostname: Option<String>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidWorkflowFile {
        message: message.into(),
    }
}

impl ProviderArgs {
    /// The PostgreSQL connection URL, if every part of it was given.
    fn postgres_url(&self) -> Result<String> {
        let part = |value: &Option<String>, flag: &str, var: &str| {
            value.clone().ok_or_else(|| {
                invalid(format!(
                    "PostgreSQL provider requires --{flag} parameter or {var} env var"
                ))
            })
        };
        Ok(format!(
            "postgresql://{}:{}@{}/{}",
            part(&self.postgres_user, "postgres-user", "POSTGRES_USER")?,
            part(
                &self.postgres_password,
                "postgres-password",
                "POSTGRES_PASSWORD"
            )?,
            part(
                &self.postgres_hostname,
                "postgres-hostname",
                "POSTGRES_HOSTNAME"
            )?,
            part(
                &self.postgres_db_name,
                "postgres-db-name",
                "POSTGRES_DB_NAME"
            )?,
        ))
    }

    fn sqlite_url(&self, what: &str) -> Result<&String> {
        self.sqlite_db_url.as_ref().ok_or_else(|| {
            invalid(format!(
                "SQLite {what} provider requires --sqlite-db-url parameter"
            ))
        })
    }

    pub async fn persistence(
        &self,
        config: &JackdawConfig,
    ) -> Result<Arc<dyn PersistenceProvider>> {
        Ok(match self.persistence_provider.as_str() {
            "memory" => Arc::new(InMemoryPersistence::new()),
            "redb" => {
                let path = config
                    .durable_db
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("workflow.db"));
                Arc::new(RedbPersistence::new(
                    path.to_str().unwrap_or("workflow.db"),
                )?)
            }
            "sqlite" => Arc::new(SqlitePersistence::new(self.sqlite_url("persistence")?).await?),
            "postgres" => Arc::new(PostgresPersistence::new(&self.postgres_url()?).await?),
            other => {
                return Err(invalid(format!(
                    "Invalid persistence provider '{other}'. Valid options: memory, redb, sqlite, postgres"
                )));
            }
        })
    }

    pub async fn cache(&self, config: &JackdawConfig) -> Result<Arc<dyn CacheProvider>> {
        Ok(match self.cache_provider.as_str() {
            "memory" => Arc::new(InMemoryCache::new()),
            "redb" => {
                let path = config
                    .cache_db
                    .as_ref()
                    .and_then(|path| path.to_str())
                    .unwrap_or("cache.db");
                let store = RedbPersistence::new(path)?;
                Arc::new(RedbCache::new(store.db.clone())?)
            }
            "sqlite" => Arc::new(SqliteCache::new(self.sqlite_url("cache")?).await?),
            "postgres" => Arc::new(PostgresCache::new(&self.postgres_url()?).await?),
            other => {
                return Err(invalid(format!(
                    "Invalid cache provider '{other}'. Valid options: memory, redb, sqlite, postgres"
                )));
            }
        })
    }
}
