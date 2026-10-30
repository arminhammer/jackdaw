//! # Errors
//!
//! There is one error type for the whole library, [`Error`], plus two that exist at the edges of your own code:
//! [`TaskError`] (what a task returns) and [`StepError`] (what a step gives a workflow). A [`Fault`] is the structured
//! failure they all carry.
//!
//! ## What each is for
//!
//! | Type | Where you meet it | Meaning |
//! |---|---|---|
//! | [`TaskError`] | the `Err` of a `#[task]` | A task's failure, and whether it is worth retrying. |
//! | [`StepError`] | the `Err` of `ctx.call(..).await` | A step failed: a [`Fault`] from a task, or an engine problem. |
//! | [`WorkflowError`] | the `Err` of `#[run]` | What a workflow fails with. Built from a `StepError` or a `Fault` by `?` / `.into()`. |
//! | [`Error`] | `execute`, `.await` on a handle, `build` | The outcome of a run or the engine. |
//!
//! ## Faults
//!
//! A [`Fault`] has the fields of the specification's error (RFC 7807): a `kind` URI, an optional `status`, `title`,
//! `detail` and `instance`. The standard kinds are in [`error_types`](crate::api::error_types). Because they are
//! the spec's, a YAML workflow can `try`/`catch` a fault raised by a Rust task and filter it by type or status.
//!
//! A task raises one with the builder on [`TaskError`]:
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task]
//! async fn charge(cents: u64) -> Result<String, TaskError> {
//!     if cents == 0 {
//!         return Err(TaskError::fault(error_types::VALIDATION)
//!             .status(400)
//!             .title("Nothing to charge")
//!             .detail("the amount is zero"));
//!     }
//!     Ok(format!("ch-{cents}"))
//! }
//! # fn main() {}
//! ```
//!
//! A workflow sees it as a [`StepError::Fault`] and can branch on it (a complete example is in
//! [workflows](super::workflows#steps)).
//!
//! ## Matching the outcome of a run
//!
//! `handle.await` gives a `Result<Out, Error>`. A workflow that faults is an `Err`, not a successful output:
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task]
//! async fn refuse(_: ()) -> Result<(), TaskError> {
//!     Err(TaskError::fault(error_types::AUTHORIZATION).status(403).detail("not allowed"))
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().task(Refuse).build()?;
//!
//! match engine.run_task(Refuse, (), TaskOptions::new()).await {
//!     Err(Error::Faulted(fault)) => assert_eq!(fault.status, Some(403)),
//!     other => panic!("unexpected: {other:?}"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`Error`] is `#[non_exhaustive]`, so a `match` needs a wildcard arm. It implements `std::error::Error` with
//! `source()`, and is `Send + Sync + 'static`, so it works with `?` into `Box<dyn std::error::Error>` and `anyhow`.
//!
//! | Variant | When |
//! |---|---|
//! | `Faulted(Fault)` | The workflow raised or failed with a fault. |
//! | `Cancelled` | The run was cancelled. |
//! | `Invalid(..)` | The setup is wrong: a bad definition, an unregistered task, a type that does not fit. |
//! | `NonDeterminism(..)` | A replay did not match its history, see [durability](super::durability). |
//! | `Persistence(..)`, `Cache(..)`, `Engine(..)` | Infrastructure failures, with a source. |
//!
//! ---
//!
//! **Next:** [Events](super::events), watching a run.
//!
//! [`Error`]: crate::api::Error
//! [`Fault`]: crate::api::Fault
//! [`StepError`]: crate::api::StepError
//! [`StepError::Fault`]: crate::api::StepError::Fault
//! [`TaskError`]: crate::api::TaskError
//! [`WorkflowError`]: crate::api::WorkflowError
