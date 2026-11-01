//! The YAML suites' engine: a `jackdaw::Engine` with the calls those suites were written against (start a definition,
//! wait for it with a timeout, read its events). Include it with `#[path = "yaml_engine/mod.rs"] mod yaml_engine;`.
#![allow(dead_code)]

use jackdaw::api::{Event, EventStream, ExecutionHandle, Workflow};
use jackdaw::dsl::workflow::WorkflowDefinition;
use jackdaw::providers::cache::mem::InMemoryCache;
use jackdaw::providers::persistence::InMemoryPersistence;
use jackdaw::{CacheProvider, Engine, Error, PersistenceProvider};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Builds a [`YamlEngine`] on the given store and cache (in memory by default).
#[derive(Default)]
pub struct YamlEngineBuilder {
    persistence: Option<Arc<dyn PersistenceProvider>>,
    cache: Option<Arc<dyn CacheProvider>>,
}

impl YamlEngineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_persistence(mut self, persistence: Arc<dyn PersistenceProvider>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    pub fn with_cache(mut self, cache: Arc<dyn CacheProvider>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn build(self) -> Result<YamlEngine, Error> {
        YamlEngine::new(
            self.persistence
                .unwrap_or_else(|| Arc::new(InMemoryPersistence::new())),
            self.cache.unwrap_or_else(|| Arc::new(InMemoryCache::new())),
        )
    }
}

/// An engine that runs YAML definitions.
#[derive(Clone, Debug)]
pub struct YamlEngine {
    pub engine: Engine,
}

impl YamlEngine {
    pub fn new(
        persistence: Arc<dyn PersistenceProvider>,
        cache: Arc<dyn CacheProvider>,
    ) -> Result<Self, Error> {
        Ok(Self {
            engine: Engine::builder()
                .persistence(persistence)
                .cache(cache)
                .build()?,
        })
    }

    /// Starts `definition` with `input`.
    pub async fn execute(
        &self,
        definition: WorkflowDefinition,
        input: Value,
    ) -> Result<YamlRun, Error> {
        let workflow = Workflow::<Value, Value>::from_definition(definition);
        let handle = self.engine.execute(&workflow, input).await?;
        let events = handle.events();
        Ok(YamlRun { handle, events })
    }

    /// Lets other definitions `run: workflow:` this one.
    pub async fn register_workflow(&self, definition: WorkflowDefinition) -> Result<(), Error> {
        self.engine
            .register_yaml_workflow(&Workflow::<Value, Value>::from_definition(definition))
            .await;
        Ok(())
    }
}

/// A started YAML instance.
pub struct YamlRun {
    handle: ExecutionHandle<Value>,
    events: EventStream,
}

impl YamlRun {
    pub fn instance_id(&self) -> &str {
        self.handle.id()
    }

    /// The next event of the run, `None` once it has ended.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.next().await
    }

    /// The run's output, or its error; `Error::Engine` if it takes longer than `timeout`.
    pub async fn wait_for_completion(self, timeout: Duration) -> Result<Value, Error> {
        match tokio::time::timeout(timeout, self.handle).await {
            Ok(result) => result,
            Err(_) => Err(Error::Engine(format!(
                "the workflow did not finish within {timeout:?} (timed out)"
            ))),
        }
    }
}
