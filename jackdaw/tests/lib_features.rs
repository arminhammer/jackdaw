//! BDD runner for the library feature specs in `tests/lib`.
//!
//! Scenarios tagged `@todo` describe behavior that is not built yet, and `@external` ones are covered by another
//! suite (named next to the scenario); both are skipped, as are `@docker` scenarios when Docker does not answer,
//! `@wasm` scenarios without the `wasm32-wasip2` target and wasmtime, and `@postgres` scenarios without `DATABASE_URL`. Every other scenario runs, and a step without a definition
//! fails the run.
//!
//! Pass cucumber options after `--`, for example a tag expression that replaces the default filter:
//!
//! ```text
//! cargo test --test lib_features -- --tags '@crash and not @todo'
//! ```
//!
//! Scenarios run 8 at a time; `JACKDAW_SCENARIO_CONCURRENCY` changes that. A scenario that runs longer than `JACKDAW_SCENARIO_TIMEOUT` seconds (600 by default) ends the run with an error
//! that names the scenarios still running, so a hang fails CI instead of stalling it. With
//! `JACKDAW_HANG_DEBUGGER=gdb` (or `lldb`) it also prints every thread's backtrace first, which needs a kernel that
//! lets a process be traced by its child (`/proc/sys/kernel/yama/ptrace_scope` 0).
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
        || (has_tag(tags, "wasm") && !lib_steps::wasm::available())
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

/// The scenarios running now, and when each started.
static RUNNING: std::sync::Mutex<Vec<(String, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

fn running() -> std::sync::MutexGuard<'static, Vec<(String, std::time::Instant)>> {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn scenario_key(
    feature: &cucumber::gherkin::Feature,
    scenario: &cucumber::gherkin::Scenario,
) -> String {
    let file = feature
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    format!("{file}:{}: {}", scenario.position.line, scenario.name)
}

/// Ends the process when a scenario overruns. A thread of its own, so a scenario that spins on a runtime worker
/// cannot keep it from running.
fn watchdog() {
    let limit = std::env::var("JACKDAW_SCENARIO_TIMEOUT")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs,
        );
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let overdue: Vec<String> = running()
                .iter()
                .filter(|(_, started)| started.elapsed() > limit)
                .map(|(key, started)| format!("  {key} (running for {:?})", started.elapsed()))
                .collect();
            if !overdue.is_empty() {
                eprintln!(
                    "\nscenarios ran past JACKDAW_SCENARIO_TIMEOUT ({limit:?}):\n{}",
                    overdue.join("\n")
                );
                if let Ok(debugger) = std::env::var("JACKDAW_HANG_DEBUGGER") {
                    let pid = std::process::id().to_string();
                    let args: Vec<&str> = if debugger.contains("lldb") {
                        vec!["--batch", "-p", &pid, "-o", "thread backtrace all"]
                    } else {
                        vec!["--batch", "-p", &pid, "-ex", "thread apply all bt"]
                    };
                    let _ = std::process::Command::new(&debugger).args(args).status();
                }
                std::process::exit(2);
            }
        }
    });
}

#[tokio::main]
async fn main() {
    let _interested = lib_steps::observability::keep_callsites_interested();
    watchdog();
    let concurrency = std::env::var("JACKDAW_SCENARIO_CONCURRENCY")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(8);
    LibWorld::cucumber()
        .max_concurrent_scenarios(concurrency)
        .fail_on_skipped()
        .before(|feature, _rule, scenario, _world| {
            running().push((scenario_key(feature, scenario), std::time::Instant::now()));
            Box::pin(async {})
        })
        .after(|feature, _rule, scenario, _event, world| {
            let key = scenario_key(feature, scenario);
            running().retain(|(running, _)| *running != key);
            // The scenario's Postgres database (`JACKDAW_TEST_STORE=postgres`), dropped with the engine using it.
            let database = world.and_then(|world| {
                world.engine = None;
                world.postgres_url.take()
            });
            Box::pin(async move {
                if let Some(url) = database {
                    lib_steps::common::drop_postgres_database(&url).await;
                }
            })
        })
        .filter_run("tests/lib", |feature, rule, scenario| {
            !(skipped(&feature.tags)
                || rule.is_some_and(|r| skipped(&r.tags))
                || skipped(&scenario.tags))
        })
        .await;
}
