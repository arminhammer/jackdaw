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
//! use jackdaw::durableengine::DurableEngine;
//! use jackdaw::providers::persistence::RedbPersistence;
//! use jackdaw::providers::cache::RedbCache;
//! use serverless_workflow_core::models::workflow::WorkflowDefinition;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize persistence and cache
//! let persistence = Arc::new(RedbPersistence::new("workflow.db")?);
//! let cache = Arc::new(RedbCache::new(persistence.db.clone())?);
//!
//! // Create the durable engine
//! let engine = DurableEngine::new(persistence, cache)?;
//!
//! // Parse a workflow from YAML
//! let workflow_yaml = r#"
//! document:
//!   dsl: '1.0.0-alpha1'
//!   namespace: examples
//!   name: hello-world
//! do:
//!   - sayHello:
//!       set:
//!         message: Hello World!
//! "#;
//!
//! let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml)?;
//!
//! // Execute the workflow
//! let (_instance_id, result) = engine.start_with_input(
//!     workflow,
//!     serde_json::json!({}),
//! ).await?;
//!
//! println!("Workflow result: {}", result);
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

pub mod cache;
pub mod config;
pub mod container;
pub mod context;
pub mod descriptors;
pub mod durableengine;
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
