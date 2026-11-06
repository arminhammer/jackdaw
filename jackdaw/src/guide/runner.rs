//! # Running a workflow as a program
//!
//! A program built on jackdaw usually does the same things around the engine: pick an instance id, work out whether the
//! run is new, half done or finished, refuse to continue it with different input, and show what is going on. The
//! [`runner`](crate::runner) module does them, so `main` stays short.
//!
//! ```
//! use jackdaw::prelude::*;
//! use jackdaw::runner::{Lines, RunOptions, Runner};
//!
//! #[task]
//! async fn greet(name: String) -> Result<String, TaskError> {
//!     Ok(format!("Hello, {name}!"))
//! }
//!
//! #[workflow(namespace = "examples", name = "hello", version = "1.0.0")]
//! struct Hello;
//!
//! #[workflow_methods]
//! impl Hello {
//!     #[run]
//!     async fn run(ctx: &mut WorkflowContext<Self>, name: String) -> WorkflowResult<String> {
//!         Ok(ctx.call(Greet, &name).await?)
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let engine = Engine::builder().task(Greet).workflow::<Hello>().build()?;
//! let mut runner = Runner::new(&engine).reporter(Lines::new(std::io::sink()));
//! let output = runner
//!     .run(Hello::run, "world".to_string(), RunOptions::new("hello/world"))
//!     .await?;
//! assert_eq!(output, "Hello, world!");
//! # Ok(())
//! # }
//! ```
//!
//! ## What `run` does
//!
//! `Runner::run` looks the instance up by its id and then:
//!
//! | The instance | `run` |
//! | --- | --- |
//! | does not exist | starts it |
//! | is running, with this input (a crash stopped it) | continues it from its journal; steps already done are reported as replayed |
//! | finished, with this input | returns the stored output and runs nothing |
//! | was started with other input | fails with [`Error::InputChanged`](crate::runner::Error::InputChanged) |
//! | failed or was cancelled | fails with [`Error::Ended`](crate::runner::Error::Ended) |
//!
//! [`RunOptions::restart`](crate::runner::RunOptions::restart) discards whatever is there and starts again, which is
//! what a `--restart` flag does. [`Runner::status`](crate::runner::Runner::status) tells where a run is without touching
//! it. To retry only what failed, run the workflow again under a new id: `format!("{base}/retry-{n}")`.
//!
//! ## Reporters
//!
//! What the run shows is the [`Reporter`](crate::runner::Reporter)'s business. It gets the engine's own
//! [events](crate::guide::events), plus a notice when the run starts, resumes, restarts or turns out to be finished.
//!
//! | Reporter | Shows | Needs |
//! | --- | --- | --- |
//! | [`Headless`](crate::runner::Headless) | nothing | |
//! | [`Lines`](crate::runner::Lines) | `fetch started`, `fetch done [3/10]`, `fetch retrying after attempt 1` | |
//! | [`Json`](crate::runner::Json) | one JSON object per event | |
//! | `Terminal` | a progress bar, a line per step, a summary | feature `terminal` |
//!
//! `Runner` runs Rust and YAML workflows alike (`&Workflow` from [`Workflow::from_yaml`](crate::api::Workflow::from_yaml)
//! is runnable), and the reporters show YAML tasks as they happen. A YAML workflow stored without an input comparison
//! (its input is not kept with the instance) is continued or returned whatever input it is given.
//!
//! The engine does not know how many steps a run will have. A program that does can say so with `.total(n)` on `Lines`
//! or `Terminal`, which then show `[done/total]` and a bar. `.describe(|task, output| ...)` adds a line of your own to
//! a finished step, such as `cell 12: 300 segments in 4.1s` made from the task's output.
//!
//! `Terminal` and `Lines` can also show what each step returned (`.outputs(true)`), and `Terminal` shows how long each
//! step took and which steps a resumed run did not redo. Several runs can share one display: make the first with
//! `Terminal::new()`, the others with `Terminal::in_group(&first.group())`, each `.label(..)`led.
//!
//! Implement `Reporter` for anything else: a status endpoint, a metrics exporter, a chat message. It is one method.
//!
//! ## Choosing from where the program runs
//!
//! [`Env::detect`](crate::runner::Env::detect) looks at standard error and `TERM`, and
//! [`Env::kind`](crate::runner::Env::kind) answers with [`Kind::Terminal`](crate::runner::Kind::Terminal) on a terminal
//! that can redraw, [`Kind::Lines`](crate::runner::Kind::Lines) in a pipe, in CI or on `TERM=dumb`,
//! [`Kind::Json`](crate::runner::Kind::Json) and [`Kind::Headless`](crate::runner::Kind::Headless) when the user asks.
//! [`reporter`](crate::runner::reporter) builds the reporter for a kind.
//!
//! With the `cli` feature, [`RunnerArgs`](crate::runner::RunnerArgs) adds the flags to a program that uses `clap`:
//! `--quiet`, `--json`, `--plain` and `--restart`.
//!
//! ```ignore
//! #[derive(clap::Parser)]
//! struct Cli {
//!     #[command(flatten)]
//!     runner: RunnerArgs,
//!     dir: std::path::PathBuf,
//! }
//!
//! let cli = Cli::parse();
//! let mut runner = Runner::new(&engine).reporter(cli.runner.reporter());
//! let report = runner.run(Pipeline::run, input, cli.runner.options("pipeline/today")).await?;
//! ```
//!
//! ## Features
//!
//! `Headless`, `Lines`, `Json` and `Runner` need no feature. `terminal` adds `indicatif` and `cli` adds `clap` (and
//! `terminal`); neither builds on wasm, where `reporter` and `Env` still work and `Terminal` is replaced by `Lines`.
//! A program that prefers its own flags and its own output uses `Runner` alone.
