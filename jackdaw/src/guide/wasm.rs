//! # WebAssembly
//!
//! jackdaw can be a dependency of a program that compiles to WebAssembly. The target is **`wasm32-wasip2`** (WASI 0.2),
//! which runs on wasmtime, Spin, wasmCloud and other WASI hosts. The browser target (`wasm32-unknown-unknown`) is not
//! supported yet.
//!
//! ```toml
//! [dependencies]
//! jackdaw = { package = "jackdaw-wf", version = "0.2" }
//! tokio = { version = "1", features = ["rt", "macros", "time"] }
//! ```
//!
//! ```text
//! rustup target add wasm32-wasip2
//! cargo build --target wasm32-wasip2
//! wasmtime run target/wasm32-wasip2/debug/my-program.wasm
//! ```
//!
//! The default features work as they are. A program is the same as on any other target, with one difference: tokio
//! offers only its single-threaded runtime on wasm.
//!
//! ```no_run
//! use jackdaw::prelude::*;
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
//!         Ok(ctx.call(Greet, &name).retry(Retry::exponential(3)).await?)
//!     }
//! }
//!
//! // `current_thread`: the only flavor tokio has on wasm.
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> Result<(), Error> {
//!     let engine = Engine::builder().task(Greet).workflow::<Hello>().build()?;
//!     println!("{}", engine.execute(Hello::run, "World".to_string()).await?.await?);
//!     Ok(())
//! }
//! ```
//!
//! ## What works
//!
//! - The native Rust API: tasks, workflows, retries, timeouts, durable timers, signals, queries, child workflows,
//!   [limits](crate::guide::tasks#limiting-how-many-calls-run-at-once), events, and [`testing`](crate::guide::testing).
//! - The YAML interpreter, with the task types that need no process or HTTP client. `set`, `for`, `fork`, `switch` and
//!   `call:` of your Rust tasks are covered by tests under wasmtime, which check that a workflow gives the output it
//!   gives natively.
//! - Durability with the [`Redb`](crate::api::Redb) store, in a directory the host gives the program access to
//!   (`wasmtime run --dir <path>`), and the in-memory store. A program that stops, and runs again on the same file,
//!   resumes its workflows.
//! - `tracing` and the `metrics` feature.
//!
//! ## What does not
//!
//! WebAssembly has no child processes, no sockets for a library to open, and no C libraries. What needs them is left
//! out or says so:
//!
//! | You use | On wasm |
//! |---|---|
//! | `ctx.run_shell`, `ctx.run_script`, `ctx.run_container`, `ctx.http` | not compiled: a compile error at the call |
//! | YAML `run: shell` | fails: "`run: shell` is not available on wasm" |
//! | YAML `run: script` (Python, JavaScript), `call: python`, `call: javascript` | fails: "… is not available on wasm" |
//! | YAML `call: http`, remote catalogs and `run: script` sources fetched by URL | fails: "… is not available on wasm" (`file://` sources work) |
//! | the features `sqlite`, `postgres`, `docker`, `http-listener`, `grpc-listener`, `openapi` | the build stops with "the `…` feature is not available on wasm" |
//!
//! The `http-client` feature is on by default and does nothing on wasm.
//!
//! ## One thread
//!
//! wasm has one thread, so the calls a workflow makes at once (`try_join!`, `join_all`, `ctx.for_each(..)`) take turns
//! instead of running in parallel. That suits calls that wait, and [limits](crate::guide::tasks) still cap them.
//! A `#[task(blocking)]` task runs where it is called, so nothing else runs until it returns: keep it short.
//!
//! Panics end the program on wasm, unless it is built to unwind: a panicking task does not turn into a failed step
//! there.
//!
//! ## One engine per file
//!
//! WASI has no file locks, so nothing stops two programs opening one Redb file at the same time. Use one engine per
//! file. (With a store that supports leases, [`EngineBuilder::lease`](crate::api::EngineBuilder::lease) lets several
//! engines share it safely; Redb does not need to, and WASI cannot share it.)
