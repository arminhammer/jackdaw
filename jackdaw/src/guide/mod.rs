//! A guide to writing durable workflows in Rust with jackdaw.
//!
//! The chapters build on each other, but each stands alone.
//!
//! | Chapter | Read it to learn |
//! |---|---|
//! | [`tasks`] | writing the units of work: `#[task]`, `#[tasks]`, retries, timeouts, caching |
//! | [`workflows`] | writing workflows in Rust: steps, parallelism, signals, queries, child workflows |
//! | [`durability`] | what replay is, the rules your code follows, and how to change a running workflow |
//! | [`catalogs`] | importing versioned tasks and workflows with types |
//! | [`yaml`] | running workflows written in the Open Workflow DSL from Rust |
//! | [`testing`] | testing workflows without waiting, with replay checked for free |
//! | [`errors`] | the error types and how to match them |
//! | [`events`] | watching a run |
//! | [`config`] | the input of a workflow as one struct, read from flags, the environment and files |
//! | [`runner`] | running a workflow as a program: start, resume, restart, and show progress |
//! | [`wasm`] | using jackdaw in a program built for WebAssembly (WASI) |
//!
//! Every example here is compiled and run by `cargo test --doc`.

// The guide covers every feature, so a build without some of them cannot resolve all its links; docs.rs builds with
// all features.
#![cfg_attr(not(feature = "full"), allow(rustdoc::broken_intra_doc_links))]

pub mod catalogs;
#[cfg(feature = "config")]
pub mod config;
pub mod durability;
pub mod errors;
pub mod events;
pub mod runner;
pub mod tasks;
pub mod testing;
pub mod wasm;
pub mod workflows;
pub mod yaml;
