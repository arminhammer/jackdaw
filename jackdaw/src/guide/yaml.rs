//! # YAML workflows
//!
//! jackdaw runs the [Serverless Workflow](https://serverlessworkflow.io) DSL, and the same engine runs Rust
//! workflows. This chapter is about the YAML side from Rust code. Rust and YAML share one engine, one event log and
//! one store.
//!
//! ## Bind a definition to Rust types
//!
//! [`Workflow<In, Out>`](crate::api::Workflow) loads a definition with [`from_yaml`](crate::api::Workflow::from_yaml),
//! [`from_json`](crate::api::Workflow::from_json) or [`from_path`](crate::api::Workflow::from_path) and ties it to the
//! Rust types of its input and output. Run it with `execute`, passing a reference:
//!
//! ```
//! use jackdaw::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct Name {
//!     name: String,
//! }
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize)]
//! struct Greeting {
//!     greeting: String,
//! }
//!
//! const HELLO: &str = r#"
//! document:
//!   dsl: '1.0.3'
//!   namespace: examples
//!   name: hello
//!   version: '1.0.0'
//! do:
//!   - greet:
//!       set:
//!         greeting: '${ "Hello, " + .name + "!" }'
//! "#;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().build()?;
//! let workflow = Workflow::<Name, Greeting>::from_yaml(HELLO)?;
//!
//! let greeting = engine.execute(&workflow, Name { name: "Ada".into() }).await?.await?;
//! assert_eq!(greeting, Greeting { greeting: "Hello, Ada!".into() });
//! # Ok(())
//! # }
//! ```
//!
//! There is no untyped workflow and no default for `In` and `Out`. When the shape really is unknown (a command-line tool,
//! an admin console), say so with `Workflow::<serde_json::Value, serde_json::Value>`; it is an ordinary typed
//! workflow whose types happen to be JSON values.
//!
//! The types are checked against the file's `input.schema` and `output.schema` when it has them. Otherwise they are
//! trusted, and a mismatch is a `validation` [`Fault`](crate::api::Fault) when the values are read.
//!
//! A faulted workflow, whether `raise`d in YAML or failed by a task, is an `Err`:
//! [`Error::Faulted`](crate::api::Error::Faulted), carrying the spec's error fields.
//!
//! ## Rust and YAML together
//!
//! | From | To | How |
//! |---|---|---|
//! | Rust workflow | YAML workflow | `ctx.run(&workflow, &input)` ([`run_definition`](crate::api::WorkflowContext::run_definition)) |
//! | YAML workflow | Rust task | `call: charge`, any task registered on the engine |
//! | Rust workflow | catalog entry | see [catalogs](super::catalogs) |
//!
//! A YAML workflow cannot run a Rust *workflow* as a child.
//!
//! A registered Rust task is available to YAML workflows by name. The YAML resolves `call: <name>` against the engine's
//! tasks, then the built-in call types (`http`, `openapi`, ...), then the catalogs.
//!
//! ```yaml
//! do:
//!   - take-payment:
//!       call: charge        # a Rust `#[task] async fn charge(..)`
//!       with:
//!         id: ${ .order_id }
//!         cents: ${ .total }
//! ```
//!
//! A Rust workflow, by contrast, cannot be exported to YAML: it is code.
//!
//! ---
//!
//! **Next:** [Testing](super::testing), testing workflows without waiting.
