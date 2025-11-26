mod common;
mod nested_workflow_steps;

use cucumber::World;
use qyvx::cache::CacheProvider;
use qyvx::durableengine::DurableEngine;
use qyvx::persistence::PersistenceProvider;
use qyvx::providers::cache::RedbCache;
use qyvx::providers::persistence::RedbPersistence;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::common::WorkflowStatus;

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
    pub workflow_events: Vec<qyvx::workflow::WorkflowEvent>,
    pub error_message: Option<String>,
}

impl NestedWorkflowWorld {
    async fn new() -> Result<Self, anyhow::Error> {
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
