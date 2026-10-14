//! # Testing
//!
//! [`TestEngine`](crate::testing::TestEngine) is an [`Engine`](crate::api::Engine) set up for tests. It derefs to the
//! engine, so `execute`, `run_task` and the rest work unchanged. It differs in two ways:
//!
//! - **Replay is checked for free.** After a workflow finishes, the test engine replays it from the journal it just
//!   wrote and fails the run with [`Error::NonDeterminism`](crate::api::Error::NonDeterminism) if the replay takes
//!   different steps or reaches a different result. An ordinary test therefore catches non-deterministic code that the
//!   compile-time scan cannot. Turn it off with `.self_replay(false)`.
//! - **Time is virtual.** Use it from `#[tokio::test(start_paused = true)]` and sleeps and timers complete at once;
//!   [`clock().elapsed()`](crate::testing::Clock::elapsed) reports how much virtual time passed.
//!
//! (`start_paused` needs tokio's `test-util` feature in your dev-dependencies.)
//!
//! ```
//! use jackdaw::prelude::*;
//! use jackdaw::testing::TestEngine;
//! use std::time::Duration;
//!
//! #[workflow(namespace = "docs", name = "reminder", version = "1.0.0")]
//! struct Reminder;
//!
//! #[workflow_methods]
//! impl Reminder {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
//!         ctx.sleep(24.hours()).await?;
//!         Ok("time is up".to_string())
//!     }
//! }
//!
//! # #[tokio::main(flavor = "current_thread", start_paused = true)]
//! # async fn main() -> Result<(), Error> {
//! // In your own tests this is `#[tokio::test(start_paused = true)] async fn ..`.
//! let engine = TestEngine::builder().workflow::<Reminder>().build()?;
//!
//! let out = engine.execute(Reminder::run, ()).await?.await?;
//!
//! assert_eq!(out, "time is up");
//! assert!(engine.clock().elapsed() >= Duration::from_secs(24 * 3600)); // a day passed, in no real time
//! # Ok(())
//! # }
//! ```
//!
//! ## Testing a task on its own
//!
//! [`Engine::run_task`](crate::api::Engine::run_task) runs one task through the same retry, timeout and fault handling a
//! workflow step gets, so you can test those without writing a workflow. See [tasks](super::tasks) for an example.
//!
//! ## Faking the outside world
//!
//! Side effects are tasks, so replacing one is registering a different task under the same name. Register a fake in the
//! test engine and the real one in production; the workflow cannot tell the difference.
//!
//! ```
//! use jackdaw::prelude::*;
//! use jackdaw::testing::TestEngine;
//!
//! // Production would charge a card; the test charges nothing.
//! #[task(name = "charge")]
//! async fn fake_charge(cents: u64) -> Result<String, TaskError> {
//!     Ok(format!("test-charge-{cents}"))
//! }
//!
//! # #[tokio::main(flavor = "current_thread", start_paused = true)]
//! # async fn main() -> Result<(), Error> {
//! let engine = TestEngine::builder().task(FakeCharge).build()?;
//! let receipt = engine.run_task(FakeCharge, 500, TaskOptions::new()).await?;
//! assert_eq!(receipt, "test-charge-500");
//! # Ok(())
//! # }
//! ```
//!
//! ## Testing a change against old histories
//!
//! To test that a change keeps running instances alive, run the old code against a store, stop it part-way, then open
//! the same store with the new code and call [`resume`](crate::api::Engine::resume) or
//! [`replay_all`](crate::api::Engine::replay_all). See [durability](super::durability).
