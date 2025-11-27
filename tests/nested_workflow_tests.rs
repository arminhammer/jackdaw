mod common;
mod nested_workflow_steps;

use cucumber::World;
use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::Value;
use snafu::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::common::WorkflowStatus;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Test setup error: {message}"))]
    TestSetup { message: String },

    #[snafu(display("Persistence error: {source}"))]
    Persistence {
        source: jackdaw::persistence::Error,
    },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: jackdaw::cache::Error },

    #[snafu(display("Engine error: {source}"))]
    Engine {
        source: jackdaw::durableengine::Error,
    },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { source }
    }
}

impl From<jackdaw::persistence::Error> for Error {
    fn from(source: jackdaw::persistence::Error) -> Self {
        Error::Persistence { source }
    }
}

impl From<jackdaw::cache::Error> for Error {
    fn from(source: jackdaw::cache::Error) -> Self {
        Error::Cache { source }
    }
}

impl From<jackdaw::durableengine::Error> for Error {
    fn from(source: jackdaw::durableengine::Error) -> Self {
        Error::Engine { source }
    }
}

/// World for nested workflow tests
#[derive(Debug, Clone, World)]
#[world(init = Self::new)]
pub struct NestedWorkflowWorld {
    pub workflow_registry: HashMap<String, String>, // key: "namespace/name/version", value: workflow YAML
    pub workflow_input: Option<Value>,
    pub workflow_output: Option<Value>,
    pub workflow_status: Option<WorkflowStatus>,
    pub engine: Option<Arc<DurableEngine>>,
    pub persistence: Option<Arc<RedbPersistence>>,
    pub instance_id: Option<String>,
    pub workflow_events: Vec<jackdaw::workflow::WorkflowEvent>,
    pub error_message: Option<String>,
}

impl NestedWorkflowWorld {
    async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join("test.db");
        let persistence = Arc::new(RedbPersistence::new(db_path.to_str().unwrap())?);
        let cache =
            Arc::new(RedbCache::new(Arc::clone(&persistence.db))?) as Arc<dyn CacheProvider>;
        let engine = Arc::new(DurableEngine::new(
            Arc::clone(&persistence) as Arc<dyn PersistenceProvider>,
            Arc::clone(&cache),
        )?);

        Ok(Self {
            workflow_registry: HashMap::new(),
            workflow_input: None,
            workflow_output: None,
            workflow_status: None,
            engine: Some(engine),
            persistence: Some(persistence),
            instance_id: None,
            workflow_events: Vec::new(),
            error_message: None,
        })
    }
}

#[tokio::main]
async fn main() {
    NestedWorkflowWorld::cucumber()
        .run("tests/fixtures/nested-workflows/")
        .await;
}
