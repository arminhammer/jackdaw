//! # Workflows
//!
//! A workflow is ordinary async Rust that runs tasks and other workflows. Control flow is `if`, `match`, loops, `?`
//! and early `return`; jackdaw adds the durability underneath. If the process dies, the workflow is rebuilt from its
//! recorded steps and carries on where it stopped. [`durability`](super::durability) explains how, and the rules your
//! code follows to make it work.
//!
//! ## A first workflow
//!
//! A workflow is a struct with `#[workflow]`, which names and versions it, and an `impl` with `#[workflow_methods]`
//! whose `#[run]` method is the code.
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
//! #[workflow(namespace = "shop", name = "order", version = "1.0.0")]
//! struct OrderWorkflow;
//!
//! #[workflow_methods]
//! impl OrderWorkflow {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Shipment> {
//!         let charge_id = ctx.call(Charge, &order).await?;
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
//!     .await? // starts the workflow and returns its handle
//!     .await?; // waits for the result
//! assert_eq!(shipment, Shipment { tracking: "trk-ch-o-1".into() });
//! # Ok(())
//! # }
//! ```
//!
//! Things to notice:
//!
//! - `#[run]` takes its input directly and returns a [`WorkflowResult`] of the output. The macro reads both types from
//!   the signature, so `execute` is typed end to end: it only accepts an `Order` and the handle awaits to a `Shipment`.
//! - The workflow is referred to by its run method, `OrderWorkflow::run`. The same pattern names signals and queries.
//! - `?` on a step turns its [`StepError`] into the workflow's own error, so a failed task fails the workflow with
//!   [`Error::Faulted`](crate::api::Error::Faulted) unless you handle it.
//! - `build()` checks that everything `run` calls is registered. A missing task is an error when the engine is built,
//!   not when the workflow reaches that line.
//! - Awaiting twice looks odd but has a reason: the first `.await` starts the instance, so you can take the handle's
//!   [`id`](crate::api::ExecutionHandle::id), [`events`](crate::api::ExecutionHandle::events), `signal` and `cancel` before
//!   waiting for the result.
//!
//! ## Steps
//!
//! Everything on [`WorkflowContext`] that does work returns a step: a future that is also configurable until you await
//! it.
//!
//! | Method | What it does |
//! |---|---|
//! | `ctx.call(task, &input)` | Runs a task. Chain `.retry(..)` and `.timeout(..)`. |
//! | `ctx.run(Child::run, &input)` | Runs another workflow and returns its output. |
//! | `ctx.run_definition(&wf, &input)` | Runs a YAML workflow bound to types. |
//! | `ctx.for_each(&items, task)` | Runs a task for each item; one at a time, or `.concurrency(n)` at once. |
//! | `ctx.sleep(duration)` | A durable timer: it survives a restart, and it costs nothing while it waits. |
//! | `ctx.wait_for_event::<T>(name)` | Waits for [`send_event`](crate::api::ExecutionHandle::send_event). |
//! | `ctx.wait_condition(\|state\| ..)` | Waits until the workflow's state satisfies the condition. |
//! | `ctx.now()`, `ctx.random()`, `ctx.uuid()` | The clock and randomness, recorded so replay sees the same values. |
//! | `ctx.state(\|state\| ..)` | Reads the workflow's fields. |
//!
//! Failures are values you can match. Here the workflow handles one kind of failure and lets the rest propagate:
//!
//! ```
//! # use jackdaw::prelude::*;
//! # #[task] async fn charge(cents: u64) -> Result<String, TaskError> { Ok(format!("ch-{cents}")) }
//! # #[task] async fn notify(reason: String) -> Result<(), TaskError> { let _ = reason; Ok(()) }
//! # #[workflow(namespace = "docs", name = "decline", version = "1.0.0")]
//! # struct Decline;
//! # #[workflow_methods]
//! # impl Decline {
//! #[run]
//! async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
//!     match ctx.call(Charge, &cents).retry(Retry::exponential(5)).timeout(30.secs()).await {
//!         Ok(charge_id) => Ok(charge_id),
//!         Err(StepError::Fault(fault)) if fault.status == Some(402) => {
//!             ctx.call(Notify, &"card declined".to_string()).await?;
//!             Err(fault.into())
//!         }
//!         Err(other) => Err(other.into()),
//!     }
//! }
//! # }
//! # fn main() {}
//! ```
//!
//! ## Running steps in parallel
//!
//! A step takes its place in the workflow's history when you **call** the method, not when the future is first polled.
//! That makes the ordinary `futures` combinators safe, and they need no jackdaw wrappers: `futures::join!`,
//! `try_join!` and `join_all` all replay correctly.
//!
//! ```
//! # use jackdaw::prelude::*;
//! # #[task] async fn price(item: String) -> Result<u64, TaskError> { Ok(item.len() as u64) }
//! # #[task] async fn stock(item: String) -> Result<u64, TaskError> { Ok(item.len() as u64 * 2) }
//! # #[workflow(namespace = "docs", name = "both", version = "1.0.0")]
//! # struct Both;
//! # #[workflow_methods]
//! # impl Both {
//! #[run]
//! async fn run(ctx: &mut WorkflowContext<Self>, item: String) -> WorkflowResult<(u64, u64)> {
//!     let (price, stock) = futures::try_join!(ctx.call(Price, &item), ctx.call(Stock, &item))?;
//!     Ok((price, stock))
//! }
//! # }
//! # fn main() {}
//! ```
//!
//! For bounded parallelism over a list use `ctx.for_each(&items, task).concurrency(8)`. Avoid combinators that start
//! work in completion order, such as `buffer_unordered` over `ctx` calls: the order of steps would depend on timing.
//!
//! ### Racing: `jackdaw::select!`
//!
//! A race is the one case that needs help, because *which branch wins* is a fact that must be recorded. Use
//! [`select!`](crate::select), which journals the winner so a replay takes the same branch and never starts the loser.
//! It is how you give a wait a deadline:
//!
//! ```
//! # use jackdaw::prelude::*;
//! # #[derive(serde::Serialize, serde::Deserialize)] struct Approved { by: String }
//! # #[workflow(namespace = "docs", name = "deadline", version = "1.0.0")]
//! # struct Deadline;
//! # #[workflow_methods]
//! # impl Deadline {
//! #[run]
//! async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
//!     Ok(jackdaw::select! {
//!         approval = ctx.wait_for_event::<Approved>("approved") => format!("approved by {}", approval?.by),
//!         timer = ctx.sleep(24.hours()) => {
//!             timer?;
//!             "expired".to_string()
//!         }
//!     })
//! }
//! # }
//! # fn main() {}
//! ```
//!
//! ## State, signals and queries
//!
//! Fields of the `#[workflow]` struct are the workflow's state. **Signals** change it from outside, **queries** read it,
//! and `wait_condition` lets `run` wait for it to change. Give the state a constructor with `#[init]`; a unit struct
//! needs none.
//!
//! ```
//! use jackdaw::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize)]
//! enum Stage {
//!     Waiting,
//!     Approved,
//! }
//!
//! #[workflow(namespace = "docs", name = "approval", version = "1.0.0")]
//! struct Approval {
//!     approved: bool,
//! }
//!
//! #[workflow_methods]
//! impl Approval {
//!     #[init]
//!     fn new(_ctx: &WorkflowContextView) -> Self {
//!         Self { approved: false }
//!     }
//!
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
//!         ctx.wait_condition(|state| state.approved).await?;
//!         Ok("shipped".to_string())
//!     }
//!
//!     // Signals change state. They are fire-and-forget and cannot fail the caller.
//!     #[signal]
//!     fn approve(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
//!         self.approved = true;
//!     }
//!
//!     // Queries read state and are never recorded.
//!     #[query]
//!     fn stage(&self, _ctx: &WorkflowContextView) -> Stage {
//!         if self.approved { Stage::Approved } else { Stage::Waiting }
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().workflow::<Approval>().build()?;
//! let run = engine.execute(Approval::run, ()).await?;
//!
//! assert_eq!(run.query(Approval::stage).await?, Stage::Waiting);
//! run.signal(Approval::approve, ()).await?; // the argument type comes from the handler
//! assert_eq!(run.await?, "shipped");
//! # Ok(())
//! # }
//! ```
//!
//! Signals are recorded and re-applied in order when an instance is rebuilt, so state survives a restart.
//!
//! ## Child workflows
//!
//! Any workflow can run another one with `ctx.run`, typed through the child's run marker. Registering the parent
//! registers its children as well.
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[workflow(namespace = "docs", name = "double", version = "1.0.0")]
//! struct Double;
//!
//! #[workflow_methods]
//! impl Double {
//!     #[run]
//!     async fn run(_ctx: &mut WorkflowContext<Self>, n: u64) -> WorkflowResult<u64> {
//!         Ok(n * 2)
//!     }
//! }
//!
//! #[workflow(namespace = "docs", name = "quadruple", version = "1.0.0")]
//! struct Quadruple;
//!
//! #[workflow_methods]
//! impl Quadruple {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, n: u64) -> WorkflowResult<u64> {
//!         let once = ctx.run(Double::run, &n).await?;
//!         Ok(ctx.run(Double::run, &once).await?)
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().workflow::<Quadruple>().build()?; // registers `Double` too
//! assert_eq!(engine.execute(Quadruple::run, 3).await?.await?, 12);
//! # Ok(())
//! # }
//! ```
//!
//! A child can also be a [YAML workflow](super::yaml) or a [catalog](super::catalogs) workflow. The other direction is
//! not offered: a YAML workflow can call a Rust *task*, but it cannot run a Rust *workflow*.
//!
//! ## Cancelling
//!
//! [`ExecutionHandle::cancel`](crate::api::ExecutionHandle::cancel) stops the workflow; awaiting it afterwards gives
//! [`Error::Cancelled`](crate::api::Error::Cancelled). A deadline on the whole run is plain
//! `tokio::time::timeout(limit, handle)` outside the workflow.
//!
//! ---
//!
//! **Next:** [Durability](super::durability), what replay is and how to change a running workflow.
//!
//! [`StepError`]: crate::api::StepError
//! [`WorkflowContext`]: crate::api::WorkflowContext
//! [`WorkflowResult`]: crate::api::WorkflowResult
