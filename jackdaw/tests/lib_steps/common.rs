//! Steps shared by many features.
#[allow(unused_imports)]
use super::support::*;
use super::world::LibWorld;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::json;

#[given(regex = r"^an engine with a Redb store( and a controllable clock)?$")]
fn redb_engine(world: &mut LibWorld) {
    world.durable = true;
}

#[given("an in-memory engine")]
fn memory_engine(world: &mut LibWorld) {
    world.durable = false;
}

#[given("a durable store shared by two engine lifetimes")]
fn shared_store(world: &mut LibWorld) {
    world.durable = true;
}

#[given(regex = r"^a workflow whose run does:$")]
fn workflow_whose_run_does(world: &mut LibWorld, step: &Step) {
    let code = step
        .docstring
        .as_deref()
        .expect("the step has the workflow's code");
    world.set_fixture(super::fixture_by_code(code));
}

#[when("I execute it")]
async fn execute_it(world: &mut LibWorld) {
    let input = world.input.clone().unwrap_or_else(|| world.fixture().input);
    world.start("it", input).await;
}

#[then(regex = r#"^"(\w+)" started before "(\w+)"$"#)]
async fn started_before(world: &mut LibWorld, a: String, b: String) {
    world.settle().await;
    let first = world.probe.calls_of(&a)[0].started;
    let second = world.probe.calls_of(&b)[0].started;
    assert!(first < second, "{a} should start before {b}");
}

#[then(regex = r#"^"(\w+)" ran (\d+) times?( in total)?$"#)]
async fn ran_times(world: &mut LibWorld, task: String, times: usize) {
    world.settle().await;
    assert_eq!(world.probe.count(&task), times, "invocations of {task}");
}

#[then("no divergence is reported")]
fn no_divergence(world: &mut LibWorld) {
    let divergences = world.divergences.as_ref().expect("replay_all ran");
    assert!(divergences.is_empty(), "{divergences:?}");
}

#[when("a new engine resumes the instance")]
async fn new_engine_resumes(world: &mut LibWorld) {
    let last = world.last();
    world.resume(last).await;
}

#[then("the workflow completes")]
async fn workflow_completes(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
}

/// Drops the engine and replays every stored instance on a new one.
pub async fn replay_all(world: &mut LibWorld) {
    world.crash().await;
    world
        .notes
        .insert("sleeps".into(), json!(world.clock.sleeps_started()));
    world.mark = Some(world.probe.calls().len());
    let engine = world.engine();
    world.divergences = Some(engine.replay_all().await.expect("replay_all"));
}

#[when(regex = r#"^I (?:replay the instance|replay it|run "Engine::replay_all")$"#)]
async fn replay(world: &mut LibWorld) {
    replay_all(world).await;
}

#[then(regex = r#"^the workflow completes with "([^"]*)"$"#)]
async fn completes_with(world: &mut LibWorld, expected: String) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    let shown = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string);
    assert_eq!(shown, expected);
}

#[when(regex = r#"^I execute it with "([^"]+)"$"#)]
async fn execute_with_fields(world: &mut LibWorld, fields: String) {
    let mut input = world.fixture().input;
    let extra: serde_json::Value =
        serde_yaml::from_str(&format!("{{{fields}}}")).expect("fields like `key: value`");
    if let (Some(base), Some(extra)) = (input.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    } else {
        input = extra;
    }
    world.start("it", input).await;
}

// ---- compile errors: checked against the trybuild snapshots the `rust` test target verifies ----

fn ui_case(code: &str) -> &'static str {
    let code = code.trim_end_matches('!').trim_end_matches("()");
    match code {
        "tokio::select" => "nondeterministic_tokio_select",
        "std::time::SystemTime::now" | "SystemTime::now" => "nondeterministic_system_time",
        "tokio::time::sleep" => "nondeterministic_tokio_sleep",
        "tokio::spawn" => "nondeterministic_tokio_spawn",
        "std::fs::read_to_string" => "nondeterministic_std_fs",
        "reqwest::get" => "nondeterministic_http_client",
        other => panic!("no compile-fail case for `{other}`"),
    }
}

#[given(regex = r#"^a "\#\[run\]" body that (?:uses|calls) "([^"]+)"$"#)]
fn run_body_uses(world: &mut LibWorld, code: String) {
    world.notes.insert("ui".into(), json!(ui_case(&code)));
}

#[when("I compile it")]
fn compile_it(world: &mut LibWorld) {
    let case = world.notes["ui"].as_str().expect("a case").to_string();
    let path = format!("{}/tests/rust/ui/{case}.stderr", env!("CARGO_MANIFEST_DIR"));
    let stderr = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    world.notes.insert("stderr".into(), json!(stderr));
}

#[then(regex = r#"^the error suggests "([^"]+)"$"#)]
fn error_suggests(world: &mut LibWorld, advice: String) {
    let stderr = world.notes["stderr"].as_str().expect("compiled");
    assert!(stderr.contains(&advice), "{stderr}");
}
