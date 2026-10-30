#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

mod common;
mod listener_steps;

#[path = "yaml_engine/mod.rs"]
mod yaml_engine;

use crate::yaml_engine::YamlEngine;
use cucumber::World;
use jackdaw::cache::CacheProvider;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::Value;
use snafu::prelude::*;
use std::sync::Arc;

use crate::common::WorkflowStatus;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Test setup error: {message}"))]
    TestSetup { message: String },

    #[snafu(display("Persistence error: {source}"))]
    Persistence { source: jackdaw::persistence::Error },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: jackdaw::cache::Error },

    #[snafu(display("Engine error: {source}"))]
    Engine { source: jackdaw::Error },

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

impl From<jackdaw::Error> for Error {
    fn from(source: jackdaw::Error) -> Self {
        Error::Engine { source }
    }
}

/// World for listener integration tests (gRPC and HTTP/OpenAPI)
#[derive(Debug, Clone, World)]
#[world(init = Self::new)]
pub struct ListenerWorld {
    pub workflow_definition: Option<String>,
    pub workflow_input: Option<Value>,
    pub workflow_output: Option<Value>,
    pub workflow_status: Option<WorkflowStatus>,
    pub engine: Option<Arc<YamlEngine>>,
    pub persistence: Option<Arc<RedbPersistence>>,
    pub instance_id: Option<String>,
    pub workflow_events: Vec<jackdaw::workflow::WorkflowEvent>,

    // Listener-specific fields
    pub grpc_requests: std::collections::HashMap<String, Value>,
    pub grpc_responses: std::collections::HashMap<String, Value>,
    pub http_requests: std::collections::HashMap<String, Value>,
    pub http_responses: std::collections::HashMap<String, Value>,

    pub http_response_status: Option<u16>,

    // Workflow abort handle for cleanup
    pub abort_handle: Option<futures::future::AbortHandle>,
}

impl ListenerWorld {
    async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join("test.db");
        let persistence = Arc::new(RedbPersistence::new(db_path.to_str().unwrap())?);
        let cache =
            Arc::new(RedbCache::new(Arc::clone(&persistence.db))?) as Arc<dyn CacheProvider>;
        let engine = Arc::new(YamlEngine::new(
            Arc::clone(&persistence) as Arc<dyn PersistenceProvider>,
            Arc::clone(&cache),
        )?);

        Ok(Self {
            workflow_definition: None,
            workflow_input: None,
            workflow_output: None,
            workflow_status: None,
            engine: Some(engine),
            persistence: Some(persistence),
            instance_id: None,
            workflow_events: Vec::new(),
            grpc_requests: std::collections::HashMap::new(),
            grpc_responses: std::collections::HashMap::new(),
            http_requests: std::collections::HashMap::new(),
            http_responses: std::collections::HashMap::new(),
            http_response_status: None,
            abort_handle: None,
        })
    }
}

#[tokio::main]
async fn main() {
    // Run listener feature tests sequentially to avoid port conflicts
    // (multiple tests binding to localhost:8080 and localhost:50051)
    // Use LocalSet to support !Send futures in workflow execution
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ListenerWorld::cucumber()
                .max_concurrent_scenarios(1) // Run one scenario at a time
                .run("tests/fixtures/listeners/features/")
                .await;
        })
        .await;
}
