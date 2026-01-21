#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::wildcard_enum_match_arm)]
#![allow(clippy::expect_fun_call)]

mod common;
mod example_steps;
use crate::common::{WorkflowStatus, parse_docstring};
use cucumber::{World, given, then, when};
use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use jackdaw::workflow_source::StringSource;
use jackdaw::DurableEngineBuilder;
use serde_json::Value;
pub use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Snafu)]
pub enum TestError {
    #[snafu(display("Test setup error: {message}"))]
    Setup { message: String },

    #[snafu(display("Persistence error: {source}"))]
    Persistence { source: jackdaw::persistence::Error },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: jackdaw::cache::Error },

    #[snafu(display("Engine error: {source}"))]
    Engine {
        source: jackdaw::durableengine::Error,
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

impl From<jackdaw::persistence::Error> for TestError {
    fn from(source: jackdaw::persistence::Error) -> Self {
        TestError::Persistence { source }
    }
}

impl From<jackdaw::cache::Error> for TestError {
    fn from(source: jackdaw::cache::Error) -> Self {
        TestError::Cache { source }
    }
}

impl From<jackdaw::durableengine::Error> for TestError {
    fn from(source: jackdaw::durableengine::Error) -> Self {
        TestError::Engine { source }
    }
}

type Result<T> = std::result::Result<T, TestError>;

// World for example tests
#[derive(Debug, Clone, World)]
#[world(init = Self::new)]
pub struct ExampleWorld {
    pub workflow_definition: Option<String>,
    pub workflow_input: Option<Value>,
    pub workflow_output: Option<Value>,
    pub workflow_status: Option<WorkflowStatus>,
    pub engine: Option<Arc<DurableEngine>>,
    pub persistence: Option<Arc<RedbPersistence>>,
    pub instance_id: Option<String>,
    pub workflow_events: Vec<jackdaw::workflow::WorkflowEvent>,
}

impl ExampleWorld {
    async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join("test.db");
        let persistence = Arc::new(RedbPersistence::new(db_path.to_str().unwrap())?);
        let cache =
            Arc::new(RedbCache::new(Arc::clone(&persistence.db))?) as Arc<dyn CacheProvider>;
        let engine = Arc::new(
            DurableEngineBuilder::new()
                .with_persistence(Arc::clone(&persistence) as Arc<dyn PersistenceProvider>)
                .with_cache(Arc::clone(&cache))
                .build()?,
        );

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

#[given(expr = "the example workflow {string}")]
async fn given_example_workflow(world: &mut ExampleWorld, workflow_file: String) {
    let workflow_path = format!("submodules/specification/examples/{}", workflow_file);
    let workflow_content = std::fs::read_to_string(&workflow_path)
        .expect(&format!("Failed to read workflow file: {}", workflow_path));
    world.workflow_definition = Some(workflow_content);
}

#[given(expr = "a workflow with definition:")]
async fn given_workflow_definition(world: &mut ExampleWorld, step: &cucumber::gherkin::Step) {
    if let Some(docstring) = &step.docstring {
        world.workflow_definition = Some(parse_docstring(docstring));
    }
}

#[given(expr = "the workflow input is:")]
async fn given_workflow_input(world: &mut ExampleWorld, step: &cucumber::gherkin::Step) {
    if let Some(docstring) = &step.docstring {
        let input_yaml = parse_docstring(docstring);
        let input: Value =
            serde_yaml::from_str(&input_yaml).expect("Failed to parse workflow input");
        world.workflow_input = Some(input);
    }
}

#[when(expr = "the workflow is executed")]
async fn when_workflow_executed(world: &mut ExampleWorld) {
    let workflow_yaml = world.workflow_definition.as_ref().unwrap().clone();
    let source = StringSource::new(workflow_yaml);

    let input = world
        .workflow_input
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));

    let handle = match world.engine.as_ref().unwrap().execute(source, input).await {
        Ok(handle) => handle,
        Err(e) => {
            let error_msg = e.to_string();
            world.workflow_status = Some(WorkflowStatus::Faulted(error_msg));
            return;
        }
    };

    let instance_id = handle.instance_id().to_string();
    world.instance_id = Some(instance_id.clone());

    // Wait for workflow completion (with generous timeout for example tests)
    let result = handle.wait_for_completion(Duration::from_secs(120)).await;

    match result {
        Ok(output) => {
            world.workflow_output = Some(output);
            world.workflow_status = Some(WorkflowStatus::Completed);

            // Fetch events from persistence for assertions
            if let Ok(events) = world
                .persistence
                .as_ref()
                .unwrap()
                .get_events(&instance_id)
                .await
            {
                world.workflow_events = events;
            }
        }
        Err(e) => {
            let error_msg = e.to_string();
            world.workflow_status = Some(WorkflowStatus::Faulted(error_msg));
        }
    }
}

#[then(expr = "the workflow should complete")]
async fn then_workflow_completes(world: &mut ExampleWorld) {
    assert_eq!(
        world.workflow_status,
        Some(WorkflowStatus::Completed),
        "Expected workflow to complete, but status was: {:?}",
        world.workflow_status
    );
}

#[tokio::main]
async fn main() {
    ExampleWorld::cucumber()
        .run("tests/fixtures/examples/features/")
        .await;
}
