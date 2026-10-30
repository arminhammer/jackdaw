//! # Jackdaw
//!
//! Durable workflows in Rust. Write workflows as ordinary async functions, or in the
//! [Serverless Workflow](https://serverlessworkflow.io/) YAML DSL, and run both on one engine that records every step,
//! so a workflow that is interrupted continues where it stopped.
//!
//! - **Strongly typed.** Tasks and workflows have typed inputs and outputs, checked by the compiler. Signals, queries,
//!   child workflows and catalog imports are typed the same way.
//! - **Plain Rust.** Control flow is `if`, `match`, loops and `?`. Macros name things; they do not define a DSL.
//! - **Durable.** Steps are journaled. A crashed or restarted process resumes its workflows from the journal, and
//!   [`Engine::replay_all`](api::Engine::replay_all) tells you before you deploy whether a change would break them.
//! - **Checked at compile time.** Calls that would make a workflow non-deterministic, like `SystemTime::now()`, are
//!   compile errors with the fix in the message.
//! - **One engine for Rust and YAML.** They share events and storage and can call each other: Rust runs YAML
//!   workflows, and YAML calls Rust tasks.
//! - **Testable.** [`testing::TestEngine`] runs workflows on a virtual clock and replays each run to verify it.
//!
//! ## A workflow in a minute
//!
//! ```
//! use jackdaw::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct Order {
//!     id: String,
//!     cents: u64,
//! }
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize)]
//! struct Shipment {
//!     tracking: String,
//! }
//!
//! // A task is a typed function with side effects. Its result is recorded. `#[task]` also creates a
//! // marker named `Charge` that carries the types, which is what the workflow and the engine refer to.
//! #[task]
//! async fn charge(order: Order) -> Result<String, TaskError> {
//!     Ok(format!("ch-{}", order.id))
//! }
//!
//! #[task]
//! async fn ship(charge_id: String) -> Result<Shipment, TaskError> {
//!     Ok(Shipment { tracking: format!("trk-{charge_id}") })
//! }
//!
//! // A workflow decides which tasks run. It is deterministic, and it can be resumed.
//! #[workflow(namespace = "shop", name = "order", version = "1.0.0")]
//! struct OrderWorkflow;
//!
//! #[workflow_methods]
//! impl OrderWorkflow {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Shipment> {
//!         let charge_id = ctx
//!             .call(Charge, &order)
//!             .retry(Retry::exponential(5))
//!             .timeout(30.secs())
//!             .await?;
//!         Ok(ctx.call(Ship, &charge_id).await?)
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder()
//!     .task(Charge)
//!     .task(Ship)
//!     .workflow::<OrderWorkflow>()
//!     .build()?;
//!
//! let shipment = engine
//!     .execute(OrderWorkflow::run, Order { id: "o-1".into(), cents: 500 })
//!     .await?
//!     .await?;
//! assert_eq!(shipment.tracking, "trk-ch-o-1");
//! # Ok(())
//! # }
//! ```
//!
//! Add `.store(Redb::open("jackdaw.redb")?)` to the builder and the workflow survives the process.
//!
//! ## Where to go next
//!
//! The [`guide`] walks through the library chapter by chapter:
//!
//! - [`guide::tasks`]: `#[task]`, `#[tasks]`, retries, timeouts, caching
//! - [`guide::workflows`]: steps, parallelism, signals, queries, child workflows
//! - [`guide::durability`]: how replay works, the rules, and changing a running workflow
//! - [`guide::catalogs`]: importing tasks and workflows with types
//! - [`guide::yaml`]: running YAML workflows from Rust
//! - [`guide::testing`], [`guide::errors`], [`guide::events`]
//!
//! The reference lives in [`api`] (the engine, handles, errors, events) and [`prelude`] (the one `use` most programs
//! need).
//!
//! ## Command line
//!
//! The `jackdaw-cli` crate installs a `jackdaw` binary that runs, validates and visualizes YAML workflows:
//!
//! ```bash
//! jackdaw run workflow.yaml
//! jackdaw validate workflow.yaml
//! ```
//!
//! ## The YAML engine's modules
//!
//! The modules below ([`durableengine`], [`executor`], [`expressions`], [`persistence`], [`cache`], [`listeners`] and
//! friends) are the engine that interprets YAML workflows, and the [`api`] is built on them. Programs use [`api`]; reach
//! for these to implement a custom persistence or cache provider or an executor.

pub mod api;
pub mod builder;
pub mod cache;
pub mod container;
pub mod context;
pub mod descriptors;
pub mod durableengine;
pub mod execution_handle;
pub mod executionhistory;
pub mod executor;
pub mod expressions;
pub mod guide;
pub mod listeners;
pub mod output;
pub mod persistence;
pub mod prelude;
pub mod providers;
pub mod task_ext;
pub mod task_output;
pub mod testing;
pub mod workflow;

// Re-export commonly used types for convenience
pub use api::{Engine, EngineBuilder, Executor};
pub use builder::DurableEngineBuilder;
pub use execution_handle::ExecutionHandle;
