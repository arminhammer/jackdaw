//! # Catalogs
//!
//! A catalog is a source of versioned tasks and workflows, written as plain Serverless Workflow YAML, so other runtimes
//! can use the same entries. jackdaw lets Rust import them **with types**.
//!
//! A catalog is a directory (or a web location) laid out as `<name>/<version>/function.yaml`:
//!
//! ```text
//! catalog/
//!   add-numbers/1.0.0/function.yaml
//!   normalize/2.0.0/function.yaml
//! ```
//!
//! ## Declaring the entries you use
//!
//! Describe the entries as a trait. Each method names an entry and its version, and its signature gives the Rust
//! input and output types. The trait has no body; the macro turns each method into a typed marker.
//!
//! ```
//! use jackdaw::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize, JsonSchema)]
//! struct AddIn {
//!     a: i64,
//!     b: i64,
//! }
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
//! struct AddOut {
//!     result: i64,
//! }
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
//! struct Text {
//!     text: String,
//! }
//!
//! #[catalog(source = "file://./tests/rust/fixtures/catalog")] // or "https://host/path"
//! trait Local {
//!     #[task(name = "add-numbers", version = "1.0.0")]
//!     async fn add(input: AddIn) -> Result<AddOut, TaskError>;
//!
//!     #[workflow(name = "normalize", version = "2.0.0")]
//!     async fn normalize(input: Text) -> WorkflowResult<Text>;
//! }
//!
//! #[workflow(namespace = "docs", name = "uses-catalog", version = "1.0.0")]
//! struct UsesCatalog;
//!
//! #[workflow_methods]
//! impl UsesCatalog {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, text: Text) -> WorkflowResult<(Text, AddOut)> {
//!         // Catalog entries are used exactly like local tasks and workflows.
//!         let sum = ctx.call(Local::add, AddIn { a: 1, b: 2 }).await?;
//!         let cleaned = ctx.run(Local::normalize, &text).await?;
//!         Ok((cleaned, sum))
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder()
//!     .catalog::<Local>()
//!     .workflow::<UsesCatalog>()
//!     .build()?;
//!
//! let sum = engine
//!     .run_task(Local::add, AddIn { a: 40, b: 2 }, TaskOptions::new())
//!     .await?;
//! assert_eq!(sum, AddOut { result: 42 });
//! # Ok(())
//! # }
//! ```
//!
//! The input and output types need [`JsonSchema`] (re-exported in the prelude) so they can be checked.
//!
//! ## What is checked, and when
//!
//! [`EngineBuilder::build`](crate::api::EngineBuilder::build) loads the entries and fails with
//! [`Error::Invalid`](crate::api::Error::Invalid) if:
//!
//! - an entry or version does not exist in the catalog, or
//! - the entry's `input.schema` requires a field your Rust input type does not have.
//!
//! If an entry carries no schema, the binding is trusted, and a mismatch at run time is a `validation`
//! [`Fault`](crate::api::Fault), the same guarantee as `serde_json::from_str::<T>`.
//!
//! ---
//!
//! **Next:** [YAML workflows](super::yaml), running Serverless Workflow YAML from Rust.
//!
//! [`JsonSchema`]: crate::prelude::JsonSchema
