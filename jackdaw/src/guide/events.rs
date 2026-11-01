//! # Events
//!
//! The engine never prints. It records what happens as [`Event`]s, and you decide what to do with them: show
//! progress, log, feed metrics. (The `jackdaw` command-line tool is one such consumer.)
//!
//! ## Watching a run
//!
//! [`ExecutionHandle::events`](crate::api::ExecutionHandle::events) returns an [`EventStream`] of the run's events from
//! the beginning, and can be called as often as you like. The stream ends after the terminal event, and events keep
//! flowing while another task awaits the result.
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task]
//! async fn upper(text: String) -> Result<String, TaskError> {
//!     Ok(text.to_uppercase())
//! }
//!
//! #[workflow(namespace = "docs", name = "shout", version = "1.0.0")]
//! struct Shout;
//!
//! #[workflow_methods]
//! impl Shout {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, text: String) -> WorkflowResult<String> {
//!         Ok(ctx.call(Upper, &text).await?)
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().task(Upper).workflow::<Shout>().build()?;
//! let run = engine.execute(Shout::run, "hello".to_string()).await?;
//!
//! let mut events = run.events();
//! let mut seen = Vec::new();
//! while let Some(event) = events.next().await {
//!     seen.push(event.name());
//! }
//! assert_eq!(seen.first(), Some(&"WorkflowStarted"));
//! assert_eq!(seen.last(), Some(&"WorkflowCompleted"));
//!
//! assert_eq!(run.await?, "HELLO");
//! # Ok(())
//! # }
//! ```
//!
//! ## What an event tells you
//!
//! [`Event`] is a `#[non_exhaustive]` enum: workflow started, resumed, completed, failed and cancelled, and task
//! started, completed, faulted and retried. Every event has an [`instance_id`](crate::api::Event::instance_id) and a
//! timestamp, and [`Event::is_terminal`](crate::api::Event::is_terminal) says whether it ends the run.
//!
//! Steps that are replayed after a restart are reported again with [`replayed`](crate::api::Event::replayed) set, so a
//! consumer can tell recorded history from new work. A step is never reported twice as new.
//!
//! [`Engine::history`](crate::api::Engine::history) returns the stored events of an instance, including after a
//! restart.
//!
//! ## Logs and traces
//!
//! jackdaw never prints and installs no subscriber: it reports through [`tracing`](https://docs.rs/tracing), and your
//! subscriber decides what to show. Rust workflows open a `workflow` span per instance and a `step` span per step.
//!
//! The YAML interpreter's trace is events on the `jackdaw::yaml` target. Each has an `event` field naming what
//! happened, with JSON values as JSON text:
//!
//! | `event` | Level | Fields |
//! |---|---|---|
//! | `script_line` | `INFO` | `task`, `index`, `stream` (`stdout` or `stderr`), `line`: a line a script task printed |
//! | `listener_started`, `listener_route` | `INFO` | `protocol`, `addr`; `route`, `task`, `addr` |
//! | `task_start`, `task_skipped` | `DEBUG` | `task`, `kind` |
//! | `task_context`, `task_input`, `task_output` | `DEBUG` | `context`, `input`; `output`, `duration_ms`, `streamed` |
//! | `run_params` | `DEBUG` | `language`, `stdin`, `arguments`, `environment` |
//! | `cache_hit`, `cache_miss` | `DEBUG` | `task`, `key`, `cached_at` |
//! | `workflow_resume`, `workflow_output` | `DEBUG` | `instance`, `from_task`; `output`, `duration_ms` |
//!
//! The `DEBUG` events carry task inputs and outputs, so keep them out of shared logs if your data is sensitive.
//! With `tracing-subscriber`, `RUST_LOG=jackdaw::yaml=debug` shows them; the `jackdaw` CLI renders them with `--debug`.
//! The field set is for display and may grow.
//!
//! ---
//!
//! That is the last chapter. Back to the [guide](super) for the list of chapters.
//!
//! [`Event`]: crate::api::Event
//! [`EventStream`]: crate::api::EventStream
