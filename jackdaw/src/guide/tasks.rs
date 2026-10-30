//! # Tasks
//!
//! A task is a unit of work with a typed input and a typed output. Anything that touches the outside world (a payment
//! gateway, a database, the clock, the network) is a task; a workflow only decides which tasks run and in what order.
//! That boundary is what lets jackdaw make workflows durable: a task's result is recorded, and a workflow that is
//! rebuilt after a crash reads the record instead of doing the work again. See [`durability`](super::durability).
//!
//! ## A task from a function
//!
//! Put `#[task]` on an `async fn`. The input is its parameter (optionally preceded by a [`TaskContext`], below), and the output is
//! the `Ok` type. Both only need to implement `Serialize` and `Deserialize`.
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
//! struct Receipt {
//!     charge_id: String,
//! }
//!
//! #[task]
//! async fn charge(order: Order) -> Result<Receipt, TaskError> {
//!     Ok(Receipt { charge_id: format!("ch-{}", order.id) })
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().task(Charge).build()?;
//!
//! let receipt = engine
//!     .run_task(Charge, Order { id: "o-1".into(), cents: 500 }, TaskOptions::new())
//!     .await?;
//! assert_eq!(receipt, Receipt { charge_id: "ch-o-1".into() });
//! # Ok(())
//! # }
//! ```
//!
//! `#[task]` keeps your function (call `charge(..)` directly in a unit test) and adds a **marker**, a unit struct named
//! after it in `CamelCase`: `Charge`. The marker carries the input and output types and is what you hand to
//! [`Engine::run_task`](crate::api::Engine::run_task), `EngineBuilder::task` and `ctx.call`. The compiler checks the
//! input you pass and gives you back the output type.
//!
//! The task is registered under its function name (`"charge"`). `#[task(name = "payments.charge")]` picks another name,
//! which is also the name a YAML workflow uses in `call:`.
//!
//! ## Asking about the call
//!
//! A [`TaskContext`] as the first parameter, before the input, says which call is being served:
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task]
//! async fn deliver(ctx: TaskContext, address: String) -> Result<String, TaskError> {
//!     // The key is the same on every retry and every replay of this step, so a payment API or a
//!     // database can use it to refuse a duplicate.
//!     let key = ctx.idempotency_key().to_string();
//!     Ok(format!("{address} (attempt {}, key {key})", ctx.attempt()))
//! }
//! # fn main() {}
//! ```
//!
//! Tasks run **at least once**. If the process dies while one is running, the step runs again when the workflow
//! resumes, so a task that is not naturally idempotent should use the key.
//!
//! ## Tasks that hold state
//!
//! Tasks that share something (a client, a connection pool, a counter) are methods of a type. Put `#[tasks]` on the
//! `impl` and `#[task]` on each method. The methods take `self: Arc<Self>`.
//!
//! ```
//! use jackdaw::prelude::*;
//! use std::sync::Arc;
//!
//! struct Gateway {
//!     currency: String,
//! }
//!
//! #[tasks]
//! impl Gateway {
//!     #[task]
//!     pub async fn quote(self: Arc<Self>, cents: u64) -> Result<String, TaskError> {
//!         Ok(format!("{cents} {}", self.currency))
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! // The set is registered once, with the state it needs.
//! let engine = Engine::builder()
//!     .tasks(Gateway { currency: "EUR".into() })
//!     .build()?;
//!
//! // `Gateway::quote` is the marker; the code lives in the registered instance.
//! let quote = engine.run_task(Gateway::quote, 250, TaskOptions::new()).await?;
//! assert_eq!(quote, "250 EUR");
//! # Ok(())
//! # }
//! ```
//!
//! ## Blocking code
//!
//! `#[task(blocking)]` on a plain `fn` runs it on tokio's blocking pool, so CPU-heavy or synchronous code cannot stall
//! the runtime.
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task(blocking)]
//! fn checksum(data: Vec<u8>) -> Result<u32, TaskError> {
//!     Ok(data.iter().map(|byte| u32::from(*byte)).sum())
//! }
//! # fn main() {}
//! ```
//!
//! ## Failing, retrying and timing out
//!
//! A task returns [`TaskError`]. Say whether the failure is worth retrying:
//!
//! | You return | What happens |
//! |---|---|
//! | `TaskError::retryable("..")` | The step is retried if the caller asked for retries; otherwise it fails. |
//! | `TaskError::non_retryable("..")` | The step fails at once, whatever the retry policy. |
//! | `TaskError::fault(error_types::VALIDATION).status(400).detail("..")` | A typed fault the workflow can match, see [`errors`](super::errors). |
//! | `?` on any `std::error::Error` | A non-retryable `runtime` fault. |
//!
//! A task that panics fails as a non-retryable fault; it never takes the engine down.
//!
//! Retries and timeouts belong to the **caller**, so the same task can be strict in one workflow and patient in
//! another. A workflow chains them on the step (`ctx.call(Charge, &order).retry(Retry::exponential(5)).timeout(30.secs())`);
//! from outside a workflow, pass [`TaskOptions`]:
//!
//! ```
//! use jackdaw::prelude::*;
//! use std::sync::atomic::{AtomicU32, Ordering};
//!
//! static CALLS: AtomicU32 = AtomicU32::new(0);
//!
//! #[task]
//! async fn flaky(_: ()) -> Result<String, TaskError> {
//!     if CALLS.fetch_add(1, Ordering::SeqCst) < 2 {
//!         return Err(TaskError::retryable("temporarily unavailable"));
//!     }
//!     Ok("done".to_string())
//! }
//!
//! # #[tokio::main(flavor = "current_thread", start_paused = true)]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().task(Flaky).build()?;
//! let options = TaskOptions::new().retry(Retry::constant(1.secs()).max_attempts(5));
//!
//! assert_eq!(engine.run_task(Flaky, (), options).await?, "done");
//! assert_eq!(CALLS.load(Ordering::SeqCst), 3);
//! # Ok(())
//! # }
//! ```
//!
//! A task can also set the wait before its next attempt with [`TaskError::next_retry_delay`], which wins over the
//! caller's backoff.
//!
//! ## Caching
//!
//! Native tasks are **not cached**. They are side effects, and skipping one because it ran before with the same input
//! would be a bug. A pure task can opt in with `#[task(cached)]`: the engine then reuses the result for equal input,
//! across instances, using the engine's cache.
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task(cached)]
//! async fn tax_rate(country: String) -> Result<f64, TaskError> {
//!     Ok(if country == "DE" { 0.19 } else { 0.0 })
//! }
//! # fn main() {}
//! ```
//!
//! ## Calling tasks from YAML
//!
//! Every registered task can be called from a YAML workflow by name, with `call: charge`; see [`yaml`](super::yaml).
//!
//! ---
//!
//! **Next:** [Workflows](super::workflows), writing workflows in Rust.
//!
//! [`TaskContext`]: crate::api::TaskContext
//! [`TaskError`]: crate::api::TaskError
//! [`TaskError::next_retry_delay`]: crate::api::TaskError::next_retry_delay
//! [`TaskOptions`]: crate::api::TaskOptions
