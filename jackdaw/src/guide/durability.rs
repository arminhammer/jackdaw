//! # Durability, replay and changing code
//!
//! A jackdaw workflow survives crashes and restarts. This chapter explains how, the few rules your code follows so
//! that it works, and how to change a workflow that is already running. You can write workflows without reading it;
//! come back when you hit a [`Error::NonDeterminism`](crate::api::Error::NonDeterminism) or need to edit a workflow
//! that has live instances.
//!
//! ## How it works
//!
//! Every step a workflow takes (a task call, a child workflow, a timer, a wait for an event) is written to a
//! **journal** as it completes. To rebuild an instance, jackdaw runs `#[run]` again from the top. Steps that are in
//! the journal return their recorded result **without running again**; the first step that is not in the journal runs
//! for real, and the workflow carries on from there.
//!
//! Replay is the exception. In a healthy process `#[run]` is an ordinary future that keeps running. Replay happens
//! only when an instance is picked up again: after a crash or restart, with
//! [`Engine::resume`](crate::api::Engine::resume).
//!
//! ## Keeping the journal
//!
//! By default everything is in memory. Give the engine a [`Redb`](crate::api::Redb) store and instances outlive the
//! process:
//!
//! ```no_run
//! use jackdaw::prelude::*;
//!
//! # #[task] async fn charge(cents: u64) -> Result<String, TaskError> { Ok(format!("ch-{cents}")) }
//! # #[workflow(namespace = "docs", name = "pay", version = "1.0.0")]
//! # struct Pay;
//! # #[workflow_methods]
//! # impl Pay {
//! #     #[run]
//! #     async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
//! #         Ok(ctx.call(Charge, &cents).await?)
//! #     }
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder()
//!     .store(Redb::open("jackdaw.redb")?)
//!     .task(Charge)
//!     .workflow::<Pay>()
//!     .build()?;
//!
//! let run = engine.execute(Pay::run, 500).await?;
//! let id = run.id().to_string(); // keep this somewhere
//!
//! // ... the process dies, and later a new one starts with the same store and registrations:
//! let resumed = engine.resume::<Pay>(&id).await?;
//! let receipt = resumed.await?;
//! # let _ = receipt;
//! # Ok(())
//! # }
//! ```
//!
//! [`Engine::status`](crate::api::Engine::status) tells you where an instance is, and
//! [`Engine::history`](crate::api::Engine::history) returns its [events](super::events).
//!
//! Dropping the last handle to an engine stops its instances without ending them, which is what a crash looks like
//! to the store. A new engine over the same store can resume them.
//!
//! ## The rules
//!
//! Replay only works if `#[run]` takes the same steps in the same order when it runs again. That is true of any code
//! whose only inputs are its arguments and the results of its steps. So:
//!
//! - **Side effects are tasks.** Anything that touches the outside world (HTTP, files, a database, environment
//!   variables) goes in a [`#[task]`](super::tasks), whose result is recorded. This is the one rule that matters.
//! - **Time and randomness come from the context.** Use `ctx.now()`, `ctx.random()` and `ctx.uuid()`, which are
//!   recorded the first time and replayed afterwards. Use `ctx.sleep(d)` for delays and `.timeout(d)` on a step.
//! - **Race with `jackdaw::select!`**, which records the winner (see [workflows](super::workflows#racing-jackdawselect)).
//! - **No background work of your own.** A spawned task would run work the journal knows nothing about; run it as a
//!   task, or in parallel with `futures::join!`.
//!
//! ### The compiler checks them
//!
//! `#[workflow_methods]` scans each `#[run]` body and refuses to compile the common mistakes, at the offending span and
//! with the fix:
//!
//! | Forbidden in `#[run]` | Use instead |
//! |---|---|
//! | `SystemTime::now`, `Instant::now`, `Utc::now` | `ctx.now()` |
//! | `rand::random`, `thread_rng` | `ctx.random()` |
//! | `Uuid::new_v4` | `ctx.uuid()` |
//! | `tokio::spawn`, `spawn_blocking`, `spawn_local` | a task, or `futures::join!` |
//! | `tokio::time::sleep`, `timeout`, `thread::sleep` | `ctx.sleep(..)`, `.timeout(..)` |
//! | `tokio::select!` | `jackdaw::select!` |
//! | `std::fs`, `std::net`, `std::env`, `tokio::fs`, `tokio::net`, HTTP clients | a task |
//!
//! The check is a scan for known calls. It catches the usual mistakes, not every one, so the test helper in
//! [`testing`](super::testing) also replays every run to catch the rest.
//!
//! If you have audited a call and know it is safe, silence the check for that statement:
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[workflow(namespace = "docs", name = "audited", version = "1.0.0")]
//! struct Audited;
//!
//! #[workflow_methods]
//! impl Audited {
//!     #[run]
//!     async fn run(_ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
//!         // Written to a log only; it never changes which steps run.
//!         #[allow_nondeterminism]
//!         let _started = std::time::Instant::now();
//!         Ok(())
//!     }
//! }
//! # fn main() {}
//! ```
//!
//! (It is a plain attribute rather than `#[allow(jackdaw::..)]` because rustc rejects an unknown tool in `allow`
//! before a macro can see it.)
//!
//! ## When replay disagrees
//!
//! In **strict** mode, the default, a step is matched by its position, kind, name and a hash of its input. If the
//! code asks for a different step than the history recorded, the instance fails with
//! [`Error::NonDeterminism`](crate::api::Error::NonDeterminism) instead of guessing. Nothing runs against a history it
//! does not match, which is the safe behaviour when steps have side effects.
//!
//! The error is a [`Divergence`](crate::api::Divergence) that points at your code:
//!
//! ```text
//! non-deterministic replay of workflow shop/order@1.0.0 instance 7c1e..: step #1 at src/order.rs:42
//! expected `call notify`, but the history has `call charge`
//! ```
//!
//! ## Changing a workflow that has running instances
//!
//! You have three tools. Pick by how big the change is.
//!
//! ### 1. Bump the version (the default for real changes)
//!
//! A workflow's identity includes its `version`, and an instance resumes with the code registered for the version it
//! started with. For a change of behaviour, give the new code a new version and keep the old code registered until its
//! instances finish. They never need to be replay-compatible with each other.
//!
//! ```text
//! Engine::builder().workflow::<OrderV1>().workflow::<OrderV2>()   // new runs use V2, old ones drain on V1
//! ```
//!
//! ### 2. Patch in place with `ctx.patched`
//!
//! For a small change you want old instances to survive, wrap the change in
//! [`ctx.patched(id)`](crate::api::WorkflowContext::patched). It returns `true` for executions that started with the new
//! code and `false` when replaying a history that began before the change, so old runs follow the old path.
//!
//! ```
//! # use jackdaw::prelude::*;
//! # #[task] async fn audit(order: String) -> Result<(), TaskError> { let _ = order; Ok(()) }
//! # #[task] async fn charge(order: String) -> Result<String, TaskError> { Ok(order) }
//! # #[workflow(namespace = "docs", name = "patched", version = "1.0.0")]
//! # struct Order;
//! # #[workflow_methods]
//! # impl Order {
//! #[run]
//! async fn run(ctx: &mut WorkflowContext<Self>, order: String) -> WorkflowResult<String> {
//!     if ctx.patched("audit-before-charge") {
//!         ctx.call(Audit, &order).await?; // new in this release
//!     }
//!     Ok(ctx.call(Charge, &order).await?)
//! }
//! # }
//! # fn main() {}
//! ```
//!
//! ### 3. Lenient replay (an opt-in)
//!
//! `#[workflow(.., replay = "lenient")]` matches steps by kind, name and *how many times that name has been used*
//! instead of by position. Adding, removing or reordering unrelated steps then does not break replay: a step with no
//! record runs live, and a recorded step the code no longer issues is ignored. Use it only for workflows whose steps
//! are safe to run again.
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[workflow(namespace = "docs", name = "tolerant", version = "1.0.0", replay = "lenient")]
//! struct Tolerant;
//! # #[workflow_methods]
//! # impl Tolerant {
//! #     #[run]
//! #     async fn run(_ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> { Ok(()) }
//! # }
//! # fn main() {}
//! ```
//!
//! ## Check before you deploy
//!
//! [`Engine::replay_all`](crate::api::Engine::replay_all) replays every stored instance against the code you are about
//! to run and returns the [`Divergence`](crate::api::Divergence)s. Run it against a copy of production data in CI
//! before shipping a change:
//!
//! ```no_run
//! # use jackdaw::prelude::*;
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder()
//!     .store(Redb::open("copy-of-production.redb")?)
//!     // .task(..).workflow::<..>() as in production
//!     .build()?;
//!
//! let divergences = engine.replay_all().await?;
//! assert!(divergences.is_empty(), "this change breaks running workflows: {divergences:#?}");
//! # Ok(())
//! # }
//! ```
//!
//! ## What is guaranteed about tasks
//!
//! Tasks run **at least once**. A task that was running when the process died runs again on resume, so side effects
//! should be idempotent; [`TaskContext::idempotency_key`](crate::api::TaskContext::idempotency_key) is stable across
//! retries and replay of the same step. Native tasks are not cached unless they opt in with `#[task(cached)]`.
//!
//! ## Several engines on one store
//!
//! An engine assumes it is alone on its store unless it is built with
//! [`lease`](crate::api::EngineBuilder::lease). Then each instance is leased to the engine running it: the lease is
//! renewed while the engine works on the instance, and every journal write checks it first. Run as many processes as
//! you like against one Postgres (or SQLite) database:
//!
//! ```no_run
//! # use jackdaw::prelude::*;
//! # use std::time::Duration;
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder()
//!     .store(jackdaw::api::Postgres::connect("postgres://jackdaw@db/jackdaw").await?)
//!     .lease(Duration::from_secs(30))
//!     .engine_id(std::env::var("HOSTNAME").unwrap_or_default())
//!     // .task(..).workflow::<..>() as everywhere else
//!     .build()?;
//!
//! // Picks up instances whose engine died (their lease ran out); leaves the others alone.
//! engine.recover().await?;
//! # Ok(())
//! # }
//! ```
//!
//! - An instance runs on one engine at a time. [`recover`](crate::api::Engine::recover) skips instances whose lease
//!   another engine holds, and takes over those whose lease expired.
//! - An engine that stalls for longer than the lease (a long GC pause, a frozen VM) finds out on its next write: the
//!   write is refused, and the engine stops the instance without recording anything more.
//! - A handle obtained on an engine that does not run the instance still works: signals and cancellation travel through
//!   the store, and awaiting the handle returns the result once the instance ends. Queries and updates need the engine
//!   that runs the instance.
//! - Without `lease`, building a second engine on a store another engine in the process uses fails, so a store cannot
//!   be shared by accident.
//!
//! ---
//!
//! **Next:** [Catalogs](super::catalogs), importing versioned tasks and workflows with types.
