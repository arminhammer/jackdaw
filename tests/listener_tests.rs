mod common;
mod listener_steps;

use cucumber::World;
use mooose::cache::CacheProvider;
use mooose::durableengine::DurableEngine;
use mooose::persistence::PersistenceProvider;
use mooose::providers::cache::RedbCache;
use mooose::providers::persistence::RedbPersistence;
use serde_json::Value;
use std::sync::Arc;

use crate::common::WorkflowStatus;

/// World for listener integration tests (gRPC and HTTP/OpenAPI)
#[derive(Debug, Clone, World)]
#[world(init = Self::new)]
pub struct ListenerWorld {
    pub workflow_definition: Option<String>,
    pub workflow_input: Option<Value>,
    pub workflow_output: Option<Value>,
    pub workflow_status: Option<WorkflowStatus>,
    pub engine: Option<Arc<DurableEngine>>,
    pub persistence: Option<Arc<RedbPersistence>>,
    pub instance_id: Option<String>,
    pub workflow_events: Vec<mooose::workflow::WorkflowEvent>,

    // Listener-specific fields
    pub grpc_requests: std::collections::HashMap<String, Value>,
    pub grpc_responses: std::collections::HashMap<String, Value>,
    pub http_requests: std::collections::HashMap<String, Value>,
    pub http_responses: std::collections::HashMap<String, Value>,

    // Active listeners
    pub http_listener: Option<Arc<mooose::listeners::http::HttpListener>>,
    pub http_response_status: Option<u16>,
}

impl ListenerWorld {
    async fn new() -> Result<Self, anyhow::Error> {
        // Configure Python path for test fixtures
        use mooose::providers::executors::PythonExecutor;
        let python_executor = PythonExecutor::new();
        python_executor.add_python_path("tests/fixtures/listeners/handlers/python-handlers")?;

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
            http_listener: None,
            http_response_status: None,
        })
    }
}

#[tokio::main]
async fn main() {
    // Run listener feature tests sequentially to avoid port conflicts
    // (multiple tests binding to localhost:8080 and localhost:50051)
    ListenerWorld::cucumber()
        .max_concurrent_scenarios(1) // Run one scenario at a time
        .run("tests/fixtures/listeners/features/")
        .await;
}
