//! Library API tests for jackdaw: the executable specification behind the `jackdaw::guide` documentation.
//!
//! Read `README.md` in this directory for the reading order, and run one area at a time with
//! `cargo test -p jackdaw --test rust <module>::`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Tests match on `#[non_exhaustive]` enums, which needs a wildcard arm; write `Ok(step.await?)` and `err.into()` to show
// the conversions that apply when the error types differ; and index fixtures whose length the test controls.
#![allow(
    clippy::wildcard_enum_match_arm,
    clippy::needless_question_mark,
    clippy::useless_conversion,
    clippy::indexing_slicing
)]

mod api_surface;
mod builder;
mod catalogs;
mod compile_fail;
mod errors;
mod events;
mod nesting;
mod quickstart;
mod replay;
mod rust_workflows;
mod support;
mod tasks;
