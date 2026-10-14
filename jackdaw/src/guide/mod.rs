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
//! | [`yaml`] | running workflows written in the Serverless Workflow DSL from Rust |
//! | [`testing`] | testing workflows without waiting, with replay checked for free |
//! | [`errors`] | the error types and how to match them |
//! | [`events`] | watching a run |
//!
//! Every example here is compiled and run by `cargo test --doc`.

pub mod catalogs;
pub mod durability;
pub mod errors;
pub mod events;
pub mod tasks;
pub mod testing;
pub mod workflows;
pub mod yaml;
