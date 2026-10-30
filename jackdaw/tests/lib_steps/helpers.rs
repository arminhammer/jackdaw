//! interop/context_task_helpers.feature: the ctx helpers for the other task types.
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};

#[workflow(namespace = "helpers", name = "ships", version = "1")]
pub struct Ships;

#[workflow_methods]
impl Ships {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, order: String) -> WorkflowResult<String> {
        ctx.emit(
            CloudEvent::new("order.shipped", json!({ "order": order })).subject(order.clone()),
        )
        .await?;
        Ok(order)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    let mut all = vec![Fixture::of::<Ships>("helpers.ships").input(json!("o-1"))];
    all.extend(helper_fixtures());
    all
}

/// The `order.shipped` events in `events`.
fn shipped(events: &[Event]) -> Vec<CloudEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Emitted { event, .. } if event.event_type == "order.shipped" => {
                Some(event.clone())
            }
            _ => None,
        })
        .collect()
}

#[when(regex = r#"^a workflow calls "ctx\.emit\(event\)"$"#)]
async fn calls_emit(world: &mut LibWorld) {
    world.set_fixture(super::fixture("helpers.ships"));
    let run = world.start("it", json!("o-1")).await;
    assert_eq!(world.outcome(run).await.value(), &json!("o-1"));
}

#[then("the event appears once on the run's event stream and in its history")]
async fn appears_once(world: &mut LibWorld) {
    let run = world.last();
    let mut stream = world.handle(run).events();
    let mut seen = Vec::new();
    while let Some(event) = stream.next().await {
        let end = event.is_terminal();
        seen.push(event);
        if end {
            break;
        }
    }
    let streamed = shipped(&seen);
    assert_eq!(streamed.len(), 1, "{seen:?}");
    let event = &streamed[0];
    assert_eq!(event.subject.as_deref(), Some("o-1"));
    assert_eq!(event.data, json!({"order": "o-1"}));
    assert!(event.source.contains("ships"), "{event:?}");
    assert!(!event.id.is_empty());
    let id = world.runs[run].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    assert_eq!(shipped(&history), streamed);
    world
        .notes
        .insert("emitted-id".into(), Value::String(event.id.clone()));
}

#[then("replaying the instance does not emit it again")]
async fn not_emitted_again(world: &mut LibWorld) {
    super::common::replay_all(world).await;
    assert_eq!(world.divergences.as_deref().map(<[_]>::len), Some(0));
    let id = world.runs[world.last()].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    let events = shipped(&history);
    assert_eq!(events.len(), 1, "{history:?}");
    assert_eq!(
        Value::String(events[0].id.clone()),
        world.notes["emitted-id"]
    );
}

// ---------------------------------------------------------------------------------------------------------------
// ctx.run_shell, ctx.run_script, ctx.run_container, ctx.http
// ---------------------------------------------------------------------------------------------------------------

use jackdaw::api::{Container, ProcessOutput, Request, Response, Script, Shell};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[workflow(namespace = "helpers", name = "shell", version = "1")]
pub struct RunsShell;

#[workflow_methods]
impl RunsShell {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        shell: Shell,
    ) -> WorkflowResult<ProcessOutput> {
        Ok(ctx.run_shell(shell).await?)
    }
}

#[workflow(namespace = "helpers", name = "script", version = "1")]
pub struct RunsScript;

#[workflow_methods]
impl RunsScript {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        script: Script,
    ) -> WorkflowResult<ProcessOutput> {
        Ok(ctx.run_script(script).await?)
    }
}

#[workflow(namespace = "helpers", name = "container", version = "1")]
pub struct RunsContainer;

#[workflow_methods]
impl RunsContainer {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        container: Container,
    ) -> WorkflowResult<ProcessOutput> {
        Ok(ctx.run_container(container).await?)
    }
}

#[workflow(namespace = "helpers", name = "http", version = "1")]
pub struct Fetches;

#[workflow_methods]
impl Fetches {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, url: String) -> WorkflowResult<Response> {
        Ok(ctx.http(Request::get(url)).await?)
    }
}

#[workflow(namespace = "helpers", name = "http-retried", version = "1")]
pub struct FetchesWithRetry;

#[workflow_methods]
impl FetchesWithRetry {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, url: String) -> WorkflowResult<Response> {
        Ok(ctx
            .http(Request::get(url))
            .retry(Retry::exponential(5))
            .await?)
    }
}

/// Which helper a caching scenario calls, and with what.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonSchema)]
pub enum Cached {
    Http(String),
    Shell(Shell),
    Script(Script),
}

#[workflow(namespace = "helpers", name = "cached", version = "1")]
pub struct CachedHelper;

#[workflow_methods]
impl CachedHelper {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, call: Cached) -> WorkflowResult<()> {
        match call {
            Cached::Http(url) => drop(ctx.http(Request::get(url)).cached().await?),
            Cached::Shell(shell) => drop(ctx.run_shell(shell).cached().await?),
            Cached::Script(script) => drop(ctx.run_script(script).cached().await?),
        }
        Ok(())
    }
}

pub fn helper_fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<RunsShell>("helpers.shell").input(json!(Shell::new("true"))),
        Fixture::of::<RunsScript>("helpers.script").input(json!(Script::python("pass"))),
        Fixture::of::<RunsContainer>("helpers.container").input(json!(Container::new("alpine:3"))),
        Fixture::of::<Fetches>("helpers.http").input(json!("http://127.0.0.1:9")),
        Fixture::of::<FetchesWithRetry>("helpers.http-retried").input(json!("http://127.0.0.1:9")),
        Fixture::of::<CachedHelper>("helpers.cached")
            .input(json!(Cached::Http("http://127.0.0.1:9".into()))),
    ]
}

/// Gherkin keeps `\"` and `\n` as written.
fn unescape(text: &str) -> String {
    text.replace("\\\"", "\"").replace("\\n", "\n")
}

async fn run_fixture(world: &mut LibWorld, name: &str, input: Value) -> usize {
    world.set_fixture(super::fixture(name));
    world.start("it", input).await
}

#[given(regex = r#"^a mock HTTP server that returns 200 with body "(.*)" for "GET (/\w+)"$"#)]
async fn mock_ok(world: &mut LibWorld, body: String, route: String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(route.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(unescape(&body), "application/json"))
        .mount(&server)
        .await;
    world
        .notes
        .insert("url".into(), json!(format!("{}{route}", server.uri())));
    world.mock = Some(server);
}

#[given("a mock HTTP server that returns 503 twice and then 200")]
async fn mock_flaky(world: &mut LibWorld) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .with_priority(2)
        .mount(&server)
        .await;
    world
        .notes
        .insert("url".into(), json!(format!("{}/flaky", server.uri())));
    world.mock = Some(server);
}

async fn requests(world: &LibWorld) -> usize {
    world
        .mock
        .as_ref()
        .expect("a mock server")
        .received_requests()
        .await
        .map_or(0, |r| r.len())
}

#[when(regex = r#"^a workflow calls "ctx\.http\(Request::get\(url\)\)"$"#)]
async fn calls_http(world: &mut LibWorld) {
    let url = world.notes["url"].clone();
    run_fixture(world, "helpers.http", url).await;
}

#[when(regex = r#"^a workflow calls "ctx\.http\(\.\.\)\.retry\(Retry::exponential\(5\)\)"$"#)]
async fn calls_http_retried(world: &mut LibWorld) {
    let url = world.notes["url"].clone();
    run_fixture(world, "helpers.http-retried", url).await;
}

#[then("the step returns status 200 and the JSON body")]
async fn status_and_body(world: &mut LibWorld) {
    let last = world.last();
    let output = world.outcome(last).await.value().clone();
    assert_eq!(output["status"], json!(200));
    assert_eq!(output["body"], json!({"ok": true}));
}

#[then("replaying the instance does not call the server again")]
async fn not_called_again(world: &mut LibWorld) {
    assert_eq!(requests(world).await, 1);
    super::common::replay_all(world).await;
    assert_eq!(world.divergences.as_deref().map(<[_]>::len), Some(0));
    assert_eq!(requests(world).await, 1);
}

#[then(regex = r"^the step returns 200 after (\d+) requests$")]
async fn ok_after(world: &mut LibWorld, n: usize) {
    let last = world.last();
    let output = world
        .outcome_within(last, Duration::from_secs(60))
        .await
        .value()
        .clone();
    assert_eq!(output["status"], json!(200));
    assert_eq!(requests(world).await, n);
}

#[when(
    regex = r#"^a workflow calls "ctx\.run_shell\(Shell::new\(\\"(\w+)\\"\)(?:\.args\(\[(.*)\]\))?\)"$"#
)]
async fn calls_shell(world: &mut LibWorld, command: String, args: String) {
    let args: Vec<String> = if args.is_empty() {
        Vec::new()
    } else {
        args.split(',')
            .map(|arg| unescape(arg.trim()).trim_matches('"').to_string())
            .collect()
    };
    run_fixture(
        world,
        "helpers.shell",
        json!(Shell::new(command).args(args)),
    )
    .await;
}

#[given(regex = r#"^"(\w+)" is installed$"#)]
fn installed(_world: &mut LibWorld, program: String) {
    let found = std::process::Command::new(&program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(found, "these scenarios need `{program}` on the PATH");
}

#[when(regex = r#"^a workflow calls "ctx\.run_script\(Script::(\w+)\(\\"(.+)\\"\)\)"$"#)]
async fn calls_script(world: &mut LibWorld, language: String, code: String) {
    let script = match language.as_str() {
        "python" => Script::python(code),
        _ => Script::javascript(code),
    };
    run_fixture(world, "helpers.script", json!(script)).await;
}

#[given("Docker is available")]
fn docker_available(_world: &mut LibWorld) {
    // The runner only runs `@docker` scenarios when `docker info` succeeds.
}

#[when(
    regex = r#"^a workflow calls "ctx\.run_container\(Container::new\(\\"([\w:.]+)\\"\)\.command\(\[(.*)\]\)\)"$"#
)]
async fn calls_container(world: &mut LibWorld, image: String, command: String) {
    let command: Vec<String> = command
        .split(',')
        .map(|arg| unescape(arg.trim()).trim_matches('"').to_string())
        .collect();
    run_fixture(
        world,
        "helpers.container",
        json!(Container::new(image).command(command)),
    )
    .await;
}

#[then(regex = r#"^the step returns stdout "(.*)"(?: and exit code (\d+))?$"#)]
async fn returns_stdout(world: &mut LibWorld, stdout: String, code: String) {
    let last = world.last();
    let output = world
        .outcome_within(last, Duration::from_secs(120))
        .await
        .value()
        .clone();
    assert_eq!(output["stdout"], json!(unescape(&stdout)), "{output}");
    if !code.is_empty() {
        assert_eq!(
            output["exit_code"],
            json!(code.parse::<i64>().expect("a code"))
        );
    }
}

#[then("the step fails with a fault that carries the exit code")]
async fn fails_with_exit_code(world: &mut LibWorld) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.title.as_deref(), Some("exit code 1"), "{fault:?}");
}

#[given(
    regex = r#"^a workflow that calls "ctx\.(\w+)" with "\.cached\(\)" twice in two instances with equal input$"#
)]
async fn cached_twice(world: &mut LibWorld, helper: String) {
    let file = world.dir.path().join("effects.txt");
    let file = file.display().to_string();
    let call = match helper.as_str() {
        "http" => {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            let url = format!("{}/once", server.uri());
            world.mock = Some(server);
            Cached::Http(url)
        }
        "run_shell" => {
            Cached::Shell(Shell::new("sh").args(["-c", &format!("echo effect >> '{file}'")]))
        }
        _ => Cached::Script(Script::python(format!(
            "open(r'{file}', 'a').write('effect\\n')"
        ))),
    };
    world.notes.insert("effects".into(), json!(file));
    for _ in 0..2 {
        let run = run_fixture(world, "helpers.cached", json!(call)).await;
        world.outcome(run).await.value();
    }
}

#[then("the underlying effect happened once")]
async fn effect_once(world: &mut LibWorld) {
    let effects = if world.mock.is_some() {
        requests(world).await
    } else {
        let file = world.notes["effects"].as_str().expect("a file").to_string();
        std::fs::read_to_string(file)
            .expect("the effect file")
            .lines()
            .count()
    };
    assert_eq!(effects, 1);
}
