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
//! ## Extending jackdaw
//!
//! - A store of your own: implement [`PersistenceProvider`] (see the `custom_persistence` example), or [`api::Store`]
//!   for a typed store. The built-in providers are in [`providers`].
//! - A cache of your own: implement [`CacheProvider`].
//! - A call type of your own: implement [`Executor`] (see the `custom_executor` example).
//!
//! The YAML interpreter that [`api`] is built on is private to the crate.
#![warn(missing_docs)]

pub mod api;
pub mod cache;
/// The Serverless Workflow DSL (1.0.3) as Rust types.
///
/// [`dsl::workflow::WorkflowDefinition`] is the root: what [`api::Workflow::from_yaml`] parses a document into and
/// [`api::Workflow::definition`] hands back. The types mirror the
/// [DSL reference](https://github.com/serverlessworkflow/specification/blob/main/dsl-reference.md) field for field.
///
/// Vendored verbatim from `core/src/models` of the
/// [Serverless Workflow Rust SDK](https://github.com/serverlessworkflow/sdk-rust) (Apache-2.0, copyright The
/// Serverless Workflow Authors; see `src/dsl/LICENSE`, and `src/dsl/UPSTREAM` for the commit). Do not edit these
/// files; run `just update-dsl` to pull a new revision.
#[allow(missing_docs, rustdoc::all, clippy::all, clippy::pedantic, clippy::restriction)]
#[rustfmt::skip]
pub mod dsl;
pub mod guide;
pub mod persistence;
pub mod prelude;
pub mod providers;
pub mod testing;
pub mod workflow;

pub use api::{Engine, EngineBuilder, Error, ExecutionHandle, Executor};
pub use cache::CacheProvider;
pub use persistence::PersistenceProvider;

// The YAML interpreter that `api` is built on.
mod container;
mod context;
mod descriptors;
mod durableengine;
mod execution_handle;
mod executionhistory;
mod executor;
mod expressions;
mod listeners;
mod output;
mod task_ext;
mod task_output;

/// What the macros' generated code refers to. Not part of the API: it changes without notice.
// The vendored DSL refers to its own modules as `crate::models::...`.
use dsl as models;

#[doc(hidden)]
pub mod __private {
    pub use crate::api::plumbing::*;
    pub use crate::api::{
        CatalogDef, CatalogTaskRef, CatalogWorkflowRef, QueryRef, SignalRef, SyncWorkflowContext,
        TaskDef, UpdateRef, WorkflowContext, WorkflowContextView, WorkflowDef, WorkflowRef,
        WorkflowResult,
    };
}
