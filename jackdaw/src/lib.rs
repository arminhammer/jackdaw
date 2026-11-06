//! # Jackdaw
//!
//! Durable workflows in Rust. Write workflows as ordinary async functions, or in the
//! [Open Workflow](https://open-workflow-specification.org/) YAML DSL (formerly Serverless Workflow), and run both on one engine that records every step,
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
//! cargo install jackdaw-cli --locked
//! jackdaw run workflow.yaml
//! jackdaw validate workflow.yaml
//! ```
//!
//! ## Features
//!
//! | Feature | Default | What it adds |
//! |---|---|---|
//! | `yaml` | yes | The Open Workflow interpreter: `dsl`, `api::Workflow` (`from_yaml`, `validate`), `ctx.run_definition`, catalogs (`#[catalog]`). Includes `http-client`. |
//! | `redb` | yes | The `Redb` store. |
//! | `http-client` | yes | `ctx.http`, YAML `call: http`, remote catalogs. |
//! | `sqlite`, `postgres` | | The `Sqlite` and `Postgres` stores. |
//! | `docker` | | `ctx.run_container` and YAML `run: container`. |
//! | `openapi` | | YAML `call: openapi`. |
//! | `http-listener`, `grpc-listener` | | `api::Listener`: the engine serves an OpenAPI document's operations or a gRPC service, each request starting a workflow. Also YAML `listen` tasks on HTTP and gRPC. |
//! | `metrics` | | Counters and histograms through the `metrics` facade. |
//! | `full` | | All of the above. |
//!
//! Without `yaml`, the native Rust API (tasks, workflows, the engine, the in-memory store, `testing`) still works:
//! `default-features = false` builds about 70 crates instead of about 150. A YAML workflow that needs a feature the build
//! lacks, such as a `listen` task without `http-listener`, fails with an error that names the feature.
//!
//! The library also builds for `wasm32-wasip2`, with the default features; see [`guide::wasm`] for what works there.
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
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod api;
pub mod cache;
/// The Open Workflow DSL (1.0.3) as Rust types.
///
/// [`dsl::workflow::WorkflowDefinition`] is the root: what [`api::Workflow::from_yaml`] parses a document into and
/// [`api::Workflow::definition`] hands back. The types mirror the
/// [DSL reference](https://github.com/open-workflow-specification/specification/blob/main/dsl-reference.md) field for field.
///
/// Vendored verbatim from `core/src/models` of the
/// [Open Workflow Specification Rust SDK](https://github.com/open-workflow-specification/sdk-rust) (Apache-2.0, copyright The
/// Serverless Workflow Authors; see `src/dsl/LICENSE`, and `src/dsl/UPSTREAM` for the commit). Do not edit these
/// files; run `just update-dsl` to pull a new revision.
#[cfg(feature = "yaml")]
#[cfg_attr(docsrs, doc(cfg(feature = "yaml")))]
#[allow(missing_docs, rustdoc::all, clippy::all, clippy::pedantic, clippy::restriction)]
#[rustfmt::skip]
pub mod dsl;
// The SDK's own unit tests, vendored from its lib.rs by `just update-dsl`.
#[cfg(all(test, feature = "yaml"))]
#[allow(unused, clippy::all, clippy::pedantic, clippy::restriction)]
#[rustfmt::skip]
#[path = "dsl/unit_tests.rs"]
mod dsl_unit_tests;
pub mod guide;
pub mod persistence;
pub mod prelude;
pub mod providers;
pub mod runner;
pub mod testing;
pub mod workflow;

pub use api::{Engine, EngineBuilder, Error, ExecutionHandle, Executor};
pub use cache::CacheProvider;
pub use persistence::PersistenceProvider;

// The YAML interpreter that `api` is built on.
// Features that need an operating system. On wasm their dependencies are not built (see Cargo.toml), so the build stops
// here, with a message that names the feature, instead of failing inside a dependency.
macro_rules! not_on_wasm {
    ($($feature:literal: $why:literal,)*) => {$(
        #[cfg(all(target_family = "wasm", feature = $feature))]
        compile_error!(concat!(
            "the `", $feature, "` feature is not available on wasm: ", $why
        ));
    )*};
}

not_on_wasm! {
    "sqlite": "SQLite is a C library that needs an operating system; use `redb`, or your own `Store`",
    "postgres": "it needs sockets; use `redb`, or your own `Store`",
    "docker": "it needs to reach the Docker daemon",
    "http-listener": "it needs sockets to listen on",
    "grpc-listener": "it needs sockets to listen on",
    "openapi": "`call: openapi` needs an HTTP client, and wasm has none yet",
    "terminal": "it draws to a terminal, which wasm has none of",
    "cli": "it reads a command line, which wasm has none of",
}

mod blocking;
#[cfg(feature = "yaml")]
mod net;
// The YAML interpreter.
#[cfg(any(feature = "yaml", feature = "docker"))]
mod container;
#[cfg(feature = "yaml")]
mod context;
#[cfg(feature = "yaml")]
mod descriptors;
#[cfg(feature = "yaml")]
mod durableengine;
#[cfg(feature = "yaml")]
mod execution_handle;
#[cfg(feature = "yaml")]
mod executionhistory;
#[cfg(feature = "yaml")]
mod executor;
#[cfg(feature = "yaml")]
mod expressions;
#[cfg(any(feature = "yaml", feature = "http-listener", feature = "grpc-listener"))]
mod listeners;
#[cfg(feature = "yaml")]
mod output;
#[cfg(feature = "yaml")]
mod task_ext;
#[cfg(feature = "yaml")]
mod task_output;

// Compiles the README's examples as doctests, so it cannot drift from the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

// `#[catalog]` wraps its expansion in this, so a build without `yaml` gets one error naming the feature instead of
// errors about missing `__private` items.
#[cfg(feature = "yaml")]
#[doc(hidden)]
#[macro_export]
macro_rules! __catalog {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(not(feature = "yaml"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __catalog {
    ($($item:tt)*) => {
        ::core::compile_error!(
            "`#[catalog]` needs the `yaml` feature of `jackdaw-wf`: catalog entries are YAML workflow definitions"
        );
    };
}

// The vendored DSL refers to its own modules as `crate::models::...`.
#[cfg(feature = "yaml")]
use dsl as models;

/// What the macros' generated code refers to. Not part of the API: it changes without notice.
#[doc(hidden)]
pub mod __private {
    pub use crate::__catalog as catalog;
    pub use crate::api::plumbing::*;
    #[cfg(feature = "yaml")]
    pub use crate::api::{CatalogDef, CatalogTaskRef, CatalogWorkflowRef};
    pub use crate::api::{
        QueryRef, SignalRef, SyncWorkflowContext, TaskDef, UpdateRef, WorkflowContext,
        WorkflowContextView, WorkflowDef, WorkflowRef, WorkflowResult,
    };
}
