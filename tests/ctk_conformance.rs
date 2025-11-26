mod common;
mod steps;
use crate::common::{WorkflowStatus, parse_docstring};
use cucumber::{World, given, then, when};
use mooose::cache::CacheProvider;
use mooose::durableengine::DurableEngine;
use mooose::persistence::PersistenceProvider;
use mooose::providers::cache::RedbCache;
use mooose::providers::persistence::RedbPersistence;
use serde_json::Value;
use snafu::prelude::*;
pub use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::sync::Arc;

#[derive(Debug, Snafu)]
pub enum TestError {
    #[snafu(display("Test setup error: {message}"))]
    Setup { message: String },

    #[snafu(display("Persistence error: {source}"))]
    Persistence {
        source: mooose::persistence::Error,
    },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: mooose::cache::Error },

    #[snafu(display("Engine error: {source}"))]
    Engine {
        source: mooose::durableengine::Error,
    },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },
}

// Manual From implementations for error conversions
impl From<std::io::Error> for TestError {
    fn from(source: std::io::Error) -> Self {
        TestError::Io { source }
    }
}

impl From<mooose::persistence::Error> for TestError {
    fn from(source: mooose::persistence::Error) -> Self {
        TestError::Persistence { source }
    }
}

impl From<mooose::cache::Error> for TestError {
    fn from(source: mooose::cache::Error) -> Self {
        TestError::Cache { source }
    }
}

impl From<mooose::durableengine::Error> for TestError {
    fn from(source: mooose::durableengine::Error) -> Self {
        TestError::Engine { source }
    }
}

type Result<T> = std::result::Result<T, TestError>;

// Single unified World for all CTK features
#[derive(Debug, Clone, World)]
#[world(init = Self::new)]
pub struct CtKWorld {
    pub workflow_definition: Option<String>,
    pub workflow_input: Option<Value>,
    pub workflow_output: Option<Value>,
    pub workflow_status: Option<WorkflowStatus>,
    pub engine: Option<Arc<DurableEngine>>,
    pub persistence: Option<Arc<RedbPersistence>>,
    pub instance_id: Option<String>,
    pub workflow_events: Vec<mooose::workflow::WorkflowEvent>,
}

impl CtKWorld {
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
            workflow_definition: None,
            workflow_input: None,
            workflow_output: None,
            workflow_status: None,
            engine: Some(engine),
            persistence: Some(persistence),
            instance_id: None,
            workflow_events: Vec::new(),
        })
    }
}

#[given(expr = "a workflow with definition:")]
async fn given_workflow_definition(world: &mut CtKWorld, step: &cucumber::gherkin::Step) {
    if let Some(docstring) = &step.docstring {
        world.workflow_definition = Some(parse_docstring(docstring));
    }
}

#[given(expr = "given the workflow input is:")]
async fn given_workflow_input(world: &mut CtKWorld, step: &cucumber::gherkin::Step) {
    if let Some(docstring) = &step.docstring {
        let input_yaml = parse_docstring(docstring);
        let input: Value =
            serde_yaml::from_str(&input_yaml).expect("Failed to parse workflow input");
        world.workflow_input = Some(input);
    }
}

#[when(expr = "the workflow is executed")]
async fn when_workflow_executed(world: &mut CtKWorld) {
    let workflow: WorkflowDefinition =
        serde_yaml::from_str(world.workflow_definition.as_ref().unwrap()).unwrap();

    let result = if let Some(input) = &world.workflow_input {
        world
            .engine
            .as_ref()
            .unwrap()
            .start_with_input(workflow, input.clone())
            .await
            .map(|(id, _)| id)
    } else {
        world.engine.as_ref().unwrap().start(workflow).await
    };

    match result {
        Ok(instance_id) => {
            world.instance_id = Some(instance_id.clone());
            if let Ok(events) = world
                .persistence
                .as_ref()
                .unwrap()
                .get_events(&instance_id)
                .await
            {
                world.workflow_events = events.clone();
                for event in events {
                    if let mooose::workflow::WorkflowEvent::WorkflowCompleted {
                        final_data, ..
                    } = event
                    {
                        world.workflow_output = Some(final_data);
                        world.workflow_status = Some(WorkflowStatus::Completed);
                        return;
                    }
                }
            }
            world.workflow_status = Some(WorkflowStatus::Completed);
        }
        Err(e) => world.workflow_status = Some(WorkflowStatus::Faulted(e.to_string())),
    }
}

#[then(expr = "the workflow should complete")]
async fn then_workflow_completes(world: &mut CtKWorld) {
    assert_eq!(
        world.workflow_status,
        Some(WorkflowStatus::Completed),
        "Expected workflow to complete, but status was: {:?}",
        world.workflow_status
    );
}

#[tokio::main]
async fn main() {
    // Run all features together with a single consolidated summary
    CtKWorld::cucumber().run("ctk/ctk/features/").await;
}
