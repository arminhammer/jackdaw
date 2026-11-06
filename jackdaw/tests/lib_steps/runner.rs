//! operations/runner.feature: `jackdaw::runner`, the lifecycle and reporting of a run as a program.
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use jackdaw::api::WorkflowRef;
use jackdaw::prelude::*;
use jackdaw::runner::{
    Env, Headless, Json, Kind, Lines, Reporter, RunOptions, Runner, RunnerArgs, Terminal,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Steps(pub Arc<Probe>);

#[tasks]
impl Steps {
    #[task(name = "runner.greet")]
    pub async fn greet(
        self: Arc<Self>,
        ctx: TaskContext,
        who: String,
    ) -> Result<String, TaskError> {
        self.0.typed("rn_greet", &ctx, &who).await
    }

    #[task(name = "runner.shout")]
    pub async fn shout(
        self: Arc<Self>,
        ctx: TaskContext,
        text: String,
    ) -> Result<String, TaskError> {
        self.0.typed("rn_shout", &ctx, &text).await
    }

    #[task(name = "runner.explode")]
    pub async fn explode(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("rn_explode", &ctx, &()).await
    }

    #[task(name = "runner.flaky")]
    pub async fn flaky(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<String, TaskError> {
        self.0.typed("rn_flaky", &ctx, &()).await
    }

    #[task(name = "runner.prepare")]
    pub async fn prepare(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("rn_prepare", &ctx, &()).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("rn_greet", |who| {
        json!(format!("hello, {}", who.as_str().unwrap_or_default()))
    });
    probe.default_output("rn_shout", |text| {
        json!(format!("{}!", text.as_str().unwrap_or_default()))
    });
    probe.default_output("rn_explode", |_| json!(null));
    probe.default_output("rn_flaky", |_| json!("steady"));
    probe.default_output("rn_prepare", |_| json!(null));
}

#[workflow(namespace = "runner", name = "greeting", version = "1")]
pub struct Greeting;

#[workflow_methods]
impl Greeting {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, who: String) -> WorkflowResult<String> {
        let greeting = ctx.call(Steps::greet, &who).await?;
        Ok(ctx.call(Steps::shout, &greeting).await?)
    }
}

#[workflow(namespace = "runner", name = "exploding", version = "1")]
pub struct Exploding;

#[workflow_methods]
impl Exploding {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        Ok(ctx.call(Steps::explode, &()).await?)
    }
}

#[workflow(namespace = "runner", name = "flaky", version = "1")]
pub struct Flaky;

#[workflow_methods]
impl Flaky {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(ctx
            .call(Steps::flaky, &())
            .retry(Retry::constant(Duration::from_millis(10)))
            .await?)
    }
}

#[workflow(namespace = "runner", name = "waiting", version = "1")]
pub struct Waiting;

#[workflow_methods]
impl Waiting {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        ctx.call(Steps::prepare, &()).await?;
        let _: Value = ctx.wait_for_event("go").await?;
        Ok("released".to_string())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Greeting>("runner.greeting").input(json!("world")),
        Fixture::of::<Exploding>("runner.exploding"),
        Fixture::of::<Flaky>("runner.flaky"),
        Fixture::of::<Waiting>("runner.waiting"),
    ]
}

/// What the report buffer collects.
#[derive(Clone, Default)]
pub struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&lock(&self.0)).into_owned()
    }
    fn lines(&self) -> Vec<String> {
        self.text()
            .lines()
            .map(|line| line.trim().to_string())
            .collect()
    }
}

#[derive(Default)]
pub struct State {
    report: Buffer,
    runner: Option<Runner<Box<dyn Reporter>>>,
    result: Option<Result<Value, jackdaw::runner::Error>>,
    kind: Option<Kind>,
    env: Env,
    restart: bool,
    term: Option<indicatif::InMemoryTerm>,
    pair: Vec<Runner<Box<dyn Reporter>>>,
}

fn install<R: Reporter + 'static>(world: &mut LibWorld, reporter: R) {
    let engine = world.engine();
    world.runner.runner =
        Some(Runner::new(&engine).reporter(Box::new(reporter) as Box<dyn Reporter>));
}

#[given("a runner with a headless reporter")]
fn headless(world: &mut LibWorld) {
    install(world, Headless);
}

#[given("a runner with a lines reporter")]
#[when("a new engine on the same store has a runner with a lines reporter")]
fn lines(world: &mut LibWorld) {
    let out = world.runner.report.clone();
    install(world, Lines::new(out));
}

#[given(regex = r#"^a runner with a lines reporter that describes "(\w+)" output as "([^"]+)"$"#)]
fn lines_described(world: &mut LibWorld, task: String, format: String) {
    let out = world.runner.report.clone();
    let wanted = format!("runner.{task}");
    install(
        world,
        Lines::new(out).describe(move |name, output| {
            (name == wanted).then(|| format.replace("<output>", &output.to_string()))
        }),
    );
}

#[given(regex = r"^a runner with a lines reporter that expects (\d+) steps$")]
fn lines_total(world: &mut LibWorld, steps: usize) {
    let out = world.runner.report.clone();
    install(world, Lines::new(out).total(steps));
}

#[given("a runner with a lines reporter that shows outputs")]
fn lines_outputs(world: &mut LibWorld) {
    let out = world.runner.report.clone();
    install(world, Lines::new(out).outputs(true));
}

#[given("a runner with a JSON reporter")]
fn json_reporter(world: &mut LibWorld) {
    let out = world.runner.report.clone();
    install(world, Json::new(out));
}

#[given("a runner with a terminal reporter that shows outputs")]
fn terminal_outputs(world: &mut LibWorld) {
    let term = indicatif::InMemoryTerm::new(24, 120);
    world.runner.term = Some(term.clone());
    let target = indicatif::ProgressDrawTarget::term_like(Box::new(term));
    install(world, Terminal::with_target(target).outputs(true));
}

#[given(
    "two runners whose terminal reporters share one display, labelled \"first\" and \"second\""
)]
fn shared_display(world: &mut LibWorld) {
    let term = indicatif::InMemoryTerm::new(24, 120);
    world.runner.term = Some(term.clone());
    let group = indicatif::MultiProgress::with_draw_target(
        indicatif::ProgressDrawTarget::term_like(Box::new(term)),
    );
    let engine = world.engine();
    for label in ["first", "second"] {
        let reporter = Box::new(Terminal::in_group(&group).label(label)) as Box<dyn Reporter>;
        world
            .runner
            .pair
            .push(Runner::new(&engine).reporter(reporter));
    }
}

#[when(regex = r#"^both runners run the "(\w+)" workflow as "([^"]+)" and "([^"]+)"$"#)]
async fn both_run(world: &mut LibWorld, workflow: String, first: String, second: String) {
    assert_eq!(workflow, "greeting");
    let clock = world.clock.clone();
    let mut pair = std::mem::take(&mut world.runner.pair);
    let (b, a) = (
        pair.pop().expect("two runners"),
        pair.pop().expect("two runners"),
    );
    let (mut a, mut b) = (a, b);
    let run = async {
        tokio::join!(
            go::<Greeting>(&mut a, json!("world"), RunOptions::new(first)),
            go::<Greeting>(&mut b, json!("world"), RunOptions::new(second)),
        )
    };
    let pump = async {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            clock.advance_next(None);
        }
    };
    let (ra, rb) = tokio::select! { r = run => r, () = pump => unreachable!() };
    ra.expect("the first run");
    rb.expect("the second run");
}

#[given("a runner with a terminal reporter drawing to a buffer")]
#[when("a new engine on the same store has a runner with a terminal reporter drawing to a buffer")]
fn terminal_reporter(world: &mut LibWorld) {
    let term = indicatif::InMemoryTerm::new(24, 120);
    world.runner.term = Some(term.clone());
    let target = indicatif::ProgressDrawTarget::term_like(Box::new(term));
    install(world, Terminal::with_target(target));
}

/// Runs the named fixture's workflow on the runner, moving the test clock along while it waits.
async fn go<W: WorkflowDef>(
    runner: &mut Runner<Box<dyn Reporter>>,
    input: Value,
    options: RunOptions,
) -> Result<Value, jackdaw::runner::Error>
where
    W::In: DeserializeOwned,
    W::Out: Serialize,
{
    let typed: W::In = serde_json::from_value(input).expect("the input fits the workflow");
    let output = runner
        .run(WorkflowRef::<W, W::In, W::Out>::new(), typed, options)
        .await?;
    Ok(serde_json::to_value(output).expect("the output is JSON"))
}

async fn run_named(
    world: &mut LibWorld,
    workflow: &str,
    id: &str,
    input: Option<Value>,
    restart: bool,
    event: Option<&str>,
    timeout: Option<Duration>,
) {
    match workflow {
        "exploding" => world.probe.script("rn_explode", |s| {
            s.fault = Some(FaultSpec::non_retryable("boom"))
        }),
        "flaky" => world.probe.script("rn_flaky", |s| s.fail_first = 1),
        _ => {}
    }
    let fixture = super::fixture(&format!("runner.{workflow}"));
    let input = input.unwrap_or_else(|| fixture.input.clone());
    let mut options = RunOptions::new(id).restart(restart);
    if let Some(timeout) = timeout {
        options = options.timeout(timeout);
    }
    let clock = world.clock.clone();
    let engine = world.engine();
    let mut runner = world
        .runner
        .runner
        .take()
        .expect("the scenario made a runner");
    let run = async {
        match workflow {
            "greeting" => go::<Greeting>(&mut runner, input, options).await,
            "exploding" => go::<Exploding>(&mut runner, input, options).await,
            "flaky" => go::<Flaky>(&mut runner, input, options).await,
            "waiting" => go::<Waiting>(&mut runner, input, options).await,
            other => panic!("no runner workflow `{other}`"),
        }
    };
    let pump = async {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            clock.advance_next(None);
        }
    };
    let send = async {
        let Some(event) = event else {
            return std::future::pending::<()>().await;
        };
        // Let the runner find the instance first, then signal whichever engine holds it.
        tokio::time::sleep(Duration::from_millis(300)).await;
        loop {
            if let Ok(handle) = (fixture.attach)(engine.clone(), id.to_string()).await
                && handle.send_event(event, json!(null)).await.is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        std::future::pending::<()>().await
    };
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::select! {
            r = run => r,
            () = pump => unreachable!(),
            () = send => unreachable!(),
        }
    })
    .await
    .expect("the run ended within 30 seconds");
    world.runner.runner = Some(runner);
    world.runner.result = Some(result);
}

#[when(regex = r#"^the runner runs the "(\w+)" workflow as "([^"]+)"$"#)]
#[given(regex = r#"^the runner ran the "(\w+)" workflow as "([^"]+)"$"#)]
async fn runs(world: &mut LibWorld, workflow: String, id: String) {
    run_named(world, &workflow, &id, None, false, None, None).await;
}

#[when(regex = r#"^the runner runs the "(\w+)" workflow as "([^"]+)" with input "([^"]+)"$"#)]
#[given(regex = r#"^the runner ran the "(\w+)" workflow as "([^"]+)" with input "([^"]+)"$"#)]
async fn runs_with(world: &mut LibWorld, workflow: String, id: String, input: String) {
    run_named(world, &workflow, &id, Some(json!(input)), false, None, None).await;
}

#[when(regex = r#"^the runner restarts the "(\w+)" workflow as "([^"]+)" with input "([^"]+)"$"#)]
async fn restarts(world: &mut LibWorld, workflow: String, id: String, input: String) {
    run_named(world, &workflow, &id, Some(json!(input)), true, None, None).await;
}

#[when(
    regex = r#"^the runner runs the "(\w+)" workflow as "([^"]+)" and sends it the event "(\w+)"$"#
)]
async fn runs_and_sends(world: &mut LibWorld, workflow: String, id: String, event: String) {
    run_named(world, &workflow, &id, None, false, Some(&event), None).await;
}

#[when(
    regex = r#"^the runner runs the "(\w+)" workflow as "([^"]+)" with a timeout of (\d+) seconds$"#
)]
async fn runs_with_timeout(world: &mut LibWorld, workflow: String, id: String, seconds: u64) {
    let timeout = Some(Duration::from_secs(seconds));
    run_named(world, &workflow, &id, None, false, None, timeout).await;
}

#[given(
    regex = r#"^the runner started the "(\w+)" workflow as "([^"]+)" and its engine then crashed$"#
)]
async fn started_then_crashed(world: &mut LibWorld, workflow: String, id: String) {
    let fixture = super::fixture(&format!("runner.{workflow}"));
    let engine = world.engine();
    (fixture.start_with_id)(engine, id, fixture.input.clone())
        .await
        .expect("the workflow starts");
    let probe = Arc::clone(&world.probe);
    world
        .until("the first step ran", move || {
            probe.completed("rn_prepare") >= 1
        })
        .await;
    world.runner.runner = None;
    world.crash().await;
}

#[then(regex = r#"^the run's output is "([^"]*)"$"#)]
fn output_is(world: &mut LibWorld, expected: String) {
    match world.runner.result.as_ref().expect("a run") {
        Ok(output) => assert_eq!(output, &json!(expected)),
        Err(error) => panic!("the run failed: {error}"),
    }
}

#[then("nothing was reported")]
fn nothing_reported(world: &mut LibWorld) {
    assert_eq!(world.runner.report.text(), "");
}

#[then("the report says, in order:")]
fn report_in_order(world: &mut LibWorld, step: &Step) {
    let expected: Vec<String> = step
        .table
        .as_ref()
        .expect("a table")
        .rows
        .iter()
        .map(|row| row[0].trim().to_string())
        .collect();
    assert_eq!(world.runner.report.lines(), expected);
}

#[then(regex = r#"^the report contains "(.*)"$"#)]
fn report_contains(world: &mut LibWorld, text: String) {
    let text = text.replace("\\\"", "\"");
    let report = world.runner.report.text();
    assert!(
        report.contains(&text),
        "`{text}` is not in the report:\n{report}"
    );
}

#[then(regex = r#"^the report ends with "(.*)"$"#)]
fn report_ends_with(world: &mut LibWorld, text: String) {
    let last = world.runner.report.lines().pop().unwrap_or_default();
    assert_eq!(last, text);
}

#[then(regex = r#"^the runner's run fails with "(.*)"$"#)]
fn run_fails(world: &mut LibWorld, text: String) {
    match world.runner.result.as_ref().expect("a run") {
        Err(error) => assert!(error.to_string().contains(&text), "the error is `{error}`"),
        Ok(output) => panic!("the run succeeded with {output}"),
    }
}

#[then(regex = r#"^the task "(\w+)" ran (\d+) times?$"#)]
fn task_ran(world: &mut LibWorld, task: String, times: usize) {
    assert_eq!(
        world.probe.count(&format!("rn_{task}")),
        times,
        "invocations of {task}"
    );
}

fn refusal(world: &LibWorld) -> &jackdaw::runner::Error {
    world
        .runner
        .result
        .as_ref()
        .expect("a run")
        .as_ref()
        .expect_err("the run was refused")
}

#[then("the run is refused because its input changed")]
fn refused_input(world: &mut LibWorld) {
    assert!(matches!(
        refusal(world),
        jackdaw::runner::Error::InputChanged { .. }
    ));
}

#[then(regex = r#"^the refusal names the stored input "([^"]+)" and the new input "([^"]+)"$"#)]
fn refusal_names(world: &mut LibWorld, stored: String, given: String) {
    let text = refusal(world).to_string();
    assert!(text.contains(&format!("\"{stored}\"")), "{text}");
    assert!(text.contains(&format!("\"{given}\"")), "{text}");
}

#[then(regex = r#"^the run is refused because it already ended as "(\w+)"$"#)]
fn refused_ended(world: &mut LibWorld, status: String) {
    match refusal(world) {
        jackdaw::runner::Error::Ended { status: got, .. } => assert_eq!(format!("{got:?}"), status),
        other => panic!("unexpected refusal: {other}"),
    }
}

#[then("the refusal suggests a restart")]
fn suggests_restart(world: &mut LibWorld) {
    assert!(refusal(world).to_string().contains("restart"));
}

#[then(regex = r#"^the status of "([^"]+)" is none$"#)]
async fn status_none(world: &mut LibWorld, id: String) {
    let runner = world.runner.runner.as_ref().expect("a runner");
    assert_eq!(runner.status(&id).await.expect("status"), None);
}

#[then(regex = r#"^the status of "([^"]+)" is "(\w+)"$"#)]
async fn status_is(world: &mut LibWorld, id: String, status: String) {
    let runner = world.runner.runner.as_ref().expect("a runner");
    let got = runner.status(&id).await.expect("status");
    assert_eq!(got.map(|s| format!("{s:?}")), Some(status));
}

#[then(regex = r#"^every line of the report is a JSON object with an "(\w+)" field$"#)]
fn json_lines(world: &mut LibWorld, field: String) {
    let lines = world.runner.report.lines();
    assert!(!lines.is_empty(), "the report is empty");
    for line in lines {
        let value: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("`{line}`: {e}"));
        assert!(value.get(&field).is_some(), "no `{field}` in {line}");
    }
}

#[then(regex = r#"^the report's events are (.+)$"#)]
fn json_events(world: &mut LibWorld, list: String) {
    let expected: Vec<String> = list
        .split(", ")
        .map(|s| s.trim_matches('"').to_string())
        .collect();
    let got: Vec<String> = world
        .runner
        .report
        .lines()
        .iter()
        .map(|line| {
            serde_json::from_str::<Value>(line).expect("JSON")["event"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(got, expected);
}

#[when(regex = r"^the program runs with (.+)$")]
fn program_runs(world: &mut LibWorld, setting: String) {
    let env = match setting.as_str() {
        "a terminal" => Env {
            terminal: true,
            ..Env::default()
        },
        "its output piped to a file" => Env::default(),
        "a terminal and the quiet flag" => Env {
            terminal: true,
            quiet: true,
            ..Env::default()
        },
        "a terminal and the json flag" => Env {
            terminal: true,
            json: true,
            ..Env::default()
        },
        "a dumb terminal" => Env {
            terminal: true,
            dumb: true,
            ..Env::default()
        },
        other => panic!("unknown setting `{other}`"),
    };
    world.runner.kind = Some(env.kind());
}

#[then(regex = r#"^the chosen reporter is "(\w+)"$"#)]
fn chosen(world: &mut LibWorld, name: String) {
    let kind = world.runner.kind.expect("a reporter was chosen");
    assert_eq!(format!("{kind:?}").to_lowercase(), name);
}

#[when(regex = r#"^the arguments "([^"]*)" are parsed$"#)]
fn parse_args(world: &mut LibWorld, args: String) {
    let argv =
        std::iter::once("program".to_string()).chain(args.split_whitespace().map(String::from));
    let parsed = RunnerArgs::try_parse_from(argv).expect("the arguments parse");
    world.runner.restart = parsed.restart;
    world.runner.kind = Some(parsed.kind(Env {
        terminal: true,
        ..Env::default()
    }));
}

#[then("the restart flag is set")]
fn restart_set(world: &mut LibWorld) {
    assert!(world.runner.restart);
}

#[then(regex = r#"^the drawn output mentions "([^"]+)"$"#)]
fn drawn_mentions(world: &mut LibWorld, text: String) {
    let drawn = world.runner.term.as_ref().expect("a terminal").contents();
    assert!(drawn.contains(&text), "`{text}` is not in:\n{drawn}");
}

#[then(regex = r"^the drawn output ends with a summary of (\d+) steps done and none failed$")]
fn drawn_summary(world: &mut LibWorld, done: usize) {
    let drawn = world.runner.term.as_ref().expect("a terminal").contents();
    let last = drawn
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or_default();
    assert!(
        last.contains(&format!("{done} steps done")) && last.contains("0 failed"),
        "{drawn}"
    );
}
