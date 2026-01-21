//! # Jackdaw - Serverless Workflow Runtime Engine
//!
//! Jackdaw is a durable, cached, graph-based execution engine for [Serverless Workflow](https://serverlessworkflow.io/) specifications.
//!
//! ## Features
//!
//! - **Durable Execution**: Workflow state is persisted to a database, allowing recovery from failures
//! - **Smart Caching**: Task results are cached based on input hash, avoiding redundant computation
//! - **Graph-Based Execution**: Workflows are represented as directed acyclic graphs (DAGs)
//! - **JQ Expression Support**: Full support for JQ expressions in workflows with null-safe transformations
//! - **Multiple Listeners**: Support for HTTP, gRPC, and other event sources
//! - **Multiple Runtimes**: Execute custom functions in Python, JavaScript/TypeScript, and more
//! - **Parallel Execution**: Execute multiple workflows concurrently
//!
//! ## Core Modules
//!
//! - [`durableengine`] - The core execution engine with persistence and recovery
//! - [`executor`] - Task execution logic and runtime integration
//! - [`expressions`] - JQ expression evaluation with null-safe transformations
//! - [`cache`] - Smart caching system for task results
//! - [`persistence`] - Database persistence layer
//! - [`listeners`] - Event listeners (HTTP, gRPC)
//! - [`workflow`] - Workflow parsing and validation
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use jackdaw::{DurableEngineBuilder, ExecutionHandle};
//! use serverless_workflow_core::models::workflow::WorkflowDefinition;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create the durable engine with builder (uses in-memory persistence/cache by default)
//! let engine = DurableEngineBuilder::new().build()?;
//!
//! // Load and parse workflow
//! let workflow_yaml = std::fs::read_to_string("examples/hello-world.yaml")?;
//! let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)?;
//!
//! // Execute the workflow
//! let mut handle = engine.execute(workflow, serde_json::json!({})).await?;
//!
//! // Option 1: Wait for completion with timeout
//! let result = handle.wait_for_completion(Duration::from_secs(30)).await?;
//! println!("Workflow result: {}", result);
//!
//! // Option 2: Stream events as they occur
//! // while let Some(event) = handle.next_event().await {
//! //     println!("Event: {:?}", event);
//! // }
//!
//! # Ok(())
//! # }
//! ```
//!
//! ## Using Custom Persistence and Cache
//!
//! ```rust,no_run
//! use jackdaw::DurableEngineBuilder;
//! use jackdaw::providers::persistence::RedbPersistence;
//! use jackdaw::providers::cache::RedbCache;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize persistence and cache
//! let persistence = Arc::new(RedbPersistence::new("workflow.db")?);
//! let cache = Arc::new(RedbCache::new(persistence.db.clone())?);
//!
//! // Create the durable engine with custom providers
//! let engine = DurableEngineBuilder::new()
//!     .with_persistence(persistence)
//!     .with_cache(cache)
//!     .build()?;
//!
//! // Use the engine...
//! # Ok(())
//! # }
//! ```
//!
//! ## Command-Line Interface
//!
//! Jackdaw provides a command-line tool for running, validating, and visualizing workflows:
//!
//! ```bash
//! # Run a workflow
//! jackdaw run workflow.yaml
//!
//! # Validate a workflow
//! jackdaw validate workflow.yaml
//!
//! # Visualize a workflow
//! jackdaw visualize workflow.yaml -o diagram.svg
//! ```
//!
//! ## Configuration
//!
//! Jackdaw can be configured via:
//! - Configuration file (`jackdaw.yaml`)
//! - Environment variables (prefix: `JACKDAW__`)
//! - Command-line arguments
//!
//! See [`config::JackdawConfig`] for available options.

pub mod builder;
pub mod cache;
pub mod config;
pub mod container;
pub mod context;
pub mod descriptors;
pub mod durableengine;
pub mod execution_handle;
pub mod executionhistory;
pub mod executor;
pub mod expressions;
pub mod listeners;
pub mod output;
pub mod persistence;
pub mod providers;
pub mod task_ext;
pub mod task_output;
pub mod workflow;

// Re-export commonly used types for convenience
pub use builder::DurableEngineBuilder;
pub use execution_handle::ExecutionHandle;
