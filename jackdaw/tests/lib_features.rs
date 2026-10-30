//! BDD runner for the library feature specs in `tests/lib`.
//!
//! Scenarios tagged `@todo` describe behavior that is not built yet, and `@external` ones are covered by another
//! suite (named next to the scenario); both are skipped, as are `@docker` scenarios when Docker does not answer and
//! `@postgres` scenarios without `DATABASE_URL`. Every other scenario runs, and a step without a definition
//! fails the run.
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

fn skipped(tags: &[String]) -> bool {
    has_tag(tags, "todo")
        || has_tag(tags, "external")
        || (has_tag(tags, "docker") && !docker())
        || (has_tag(tags, "postgres") && std::env::var_os("DATABASE_URL").is_none())
}

/// Whether Docker answers; `@docker` scenarios are skipped without it.
fn docker() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::process::Command::new("docker")
            .arg("info")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[tokio::main]
async fn main() {
    let _interested = lib_steps::observability::keep_callsites_interested();
    LibWorld::cucumber()
        .max_concurrent_scenarios(8)
        .fail_on_skipped()
        .filter_run("tests/lib", |feature, rule, scenario| {
            !(skipped(&feature.tags)
                || rule.is_some_and(|r| skipped(&r.tags))
                || skipped(&scenario.tags))
        })
        .await;
}
