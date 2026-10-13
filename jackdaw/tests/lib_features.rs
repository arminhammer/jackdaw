//! BDD runner for the library feature specs in `tests/lib`.
//!
//! Scenarios tagged `@todo` describe behavior that is not built yet and are skipped. Every other scenario runs.
//! Steps without a definition are reported as skipped, so the suite stays green while step definitions are written;
//! add `.fail_on_skipped()` once the `@implemented` scenarios are fully wired up.
//!
//! Pass cucumber options after `--`, for example a tag expression that replaces the default filter:
//!
//! ```text
//! cargo test --test lib_features -- --tags '@crash and not @todo'
//! ```

use cucumber::World;

/// Shared state for the library scenarios. Step definitions add fields here as they are written.
#[derive(Debug, Default, World)]
pub struct LibWorld {}

fn has_tag(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|t| t.trim_start_matches('@') == tag)
}

#[tokio::main]
async fn main() {
    LibWorld::cucumber()
        .filter_run("tests/lib", |feature, rule, scenario| {
            !(has_tag(&feature.tags, "todo")
                || rule.is_some_and(|r| has_tag(&r.tags, "todo"))
                || has_tag(&scenario.tags, "todo"))
        })
        .await;
}
