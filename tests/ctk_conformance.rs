#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::wildcard_enum_match_arm)]
#![allow(clippy::expect_fun_call)]

mod common;
mod steps;
use crate::common::{WorkflowStatus, parse_docstring};
use cucumber::{World, given, then, when};
use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use serde_json::Value;
pub use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::sync::Arc;

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
    pub workflow_events: Vec<jackdaw::workflow::WorkflowEvent>,
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
                    if let jackdaw::workflow::WorkflowEvent::WorkflowCompleted {
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
        Err(e) => {
            // Extract the actual message from the error
            // For TaskExecution errors from Raise tasks, the message is JSON
            let error_msg = match &e {
                jackdaw::durableengine::Error::TaskExecution { message } => message.clone(),
                _ => e.to_string(),
            };
            world.workflow_status = Some(WorkflowStatus::Faulted(error_msg));
        }
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

/// Pet IDs that are used in CTK conformance tests
const REQUIRED_PET_IDS: &[i64] = &[1, 2];

/// Ensure that all required pets exist in the Petstore API
/// This prevents test instability due to the globally mutable Petstore API
async fn ensure_petstore_health() -> Result<()> {
    println!("Checking Petstore API health...");

    let client = reqwest::Client::new();

    for pet_id in REQUIRED_PET_IDS {
        // Check if the pet exists
        let get_url = format!("https://petstore.swagger.io/v2/pet/{}", pet_id);
        let response = client.get(&get_url).send().await;

        let needs_creation = match response {
            Ok(resp) if resp.status().is_success() => {
                // Pet exists, verify it has the required fields
                if let Ok(pet) = resp.json::<serde_json::Value>().await {
                    // Check if pet has valid data
                    if pet.get("id").is_some()
                        && pet.get("name").is_some()
                        && pet.get("status").is_some()
                    {
                        println!("  Pet {} exists and is healthy", pet_id);
                        false
                    } else {
                        println!(
                            "  Pet {} exists but has invalid data, recreating...",
                            pet_id
                        );
                        true
                    }
                } else {
                    println!(
                        "  Pet {} exists but response is invalid, recreating...",
                        pet_id
                    );
                    true
                }
            }
            Ok(resp) if resp.status() == 404 => {
                println!("  Pet {} not found, creating...", pet_id);
                true
            }
            Ok(resp) => {
                println!(
                    "  Pet {} returned status {}, recreating...",
                    pet_id,
                    resp.status()
                );
                true
            }
            Err(e) => {
                println!(
                    "  Failed to check pet {}: {}, attempting to create...",
                    pet_id, e
                );
                true
            }
        };

        if needs_creation {
            // Create or update the pet using PUT
            let pet_data = serde_json::json!({
                "id": pet_id,
                "name": format!("TestPet{}", pet_id),
                "status": "available",
                "photoUrls": ["https://example.com/photo.jpg"],
                "category": {
                    "id": 1,
                    "name": "Dogs"
                },
                "tags": [
                    {
                        "id": 1,
                        "name": "test"
                    }
                ]
            });

            let put_response = client
                .put("https://petstore.swagger.io/v2/pet")
                .header("Content-Type", "application/json")
                .json(&pet_data)
                .send()
                .await;

            match put_response {
                Ok(resp) if resp.status().is_success() => {
                    println!("  ✓ Successfully created/updated pet {}", pet_id);
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    println!(
                        "  Failed to create pet {} (status {}): {}",
                        pet_id, status, body
                    );
                    // Don't fail the tests, just warn
                }
                Err(e) => {
                    println!("  Error creating pet {}: {}", pet_id, e);
                    // Don't fail the tests, just warn
                }
            }
        }
    }

    println!("Petstore API health check complete\n");
    Ok(())
}

#[tokio::main]
async fn main() {
    // Ensure Petstore API is healthy before running tests
    if let Err(e) = ensure_petstore_health().await {
        eprintln!("Warning: Petstore health check failed: {}", e);
        eprintln!("Continuing with tests anyway...\n");
    }

    // Run all features together with a single consolidated summary
    CtKWorld::cucumber().run("ctk/ctk/features/").await;
}
