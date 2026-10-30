//! BDD runner for the library feature specs in `tests/lib`.
//!
//! Scenarios tagged `@todo` describe behavior that is not built yet and are skipped. Every other scenario runs.
//! Steps without a definition are reported as skipped; add `.fail_on_skipped()` once every `@implemented` scenario
//! is wired up.
//!
//! Pass cucumber options after `--`, for example a tag expression that replaces the default filter:
//!
//! ```text
//! cargo test --test lib_features -- --tags '@crash and not @todo'
//! ```
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::unused_async,
    // Fixtures are written the way the scenarios show them: `ctx.call(Task, &input)` and `Ok(step.await?)`.
    clippy::needless_borrows_for_generic_args,
    clippy::needless_question_mark,
    clippy::useless_conversion
)]

mod lib_steps;

use cucumber::World as _;
use lib_steps::LibWorld;

fn has_tag(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|t| t.trim_start_matches('@') == tag)
}

#[tokio::main]
async fn main() {
    LibWorld::cucumber()
        .max_concurrent_scenarios(8)
        .filter_run("tests/lib", |feature, rule, scenario| {
            !(has_tag(&feature.tags, "todo")
                || rule.is_some_and(|r| has_tag(&r.tags, "todo"))
                || has_tag(&scenario.tags, "todo"))
        })
        .await;
}
