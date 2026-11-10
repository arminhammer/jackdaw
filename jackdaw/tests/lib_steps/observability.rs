//! operations/observability.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use metrics_util::CompositeKey;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::instrument::WithSubscriber;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

pub struct Observed(pub Arc<Probe>);

#[tasks]
impl Observed {
    #[task(name = "obs.charge")]
    pub async fn charge(self: Arc<Self>, ctx: TaskContext, cents: u64) -> Result<u64, TaskError> {
        tracing::info!(cents, "charging");
        self.0.typed("obs_charge", &ctx, &cents).await
    }

    #[task(name = "obs.ship")]
    pub async fn ship(self: Arc<Self>, ctx: TaskContext, cents: u64) -> Result<u64, TaskError> {
        self.0.typed("obs_ship", &ctx, &cents).await
    }

    #[task(name = "obs.flaky")]
    pub async fn flaky(self: Arc<Self>, ctx: TaskContext, cents: u64) -> Result<u64, TaskError> {
        self.0.typed("obs_flaky", &ctx, &cents).await
    }

    #[task(name = "obs.wobbly")]
    pub async fn wobbly(self: Arc<Self>, ctx: TaskContext, cents: u64) -> Result<u64, TaskError> {
        self.0.typed("obs_wobbly", &ctx, &cents).await
    }

    #[task(name = "obs.meter")]
    pub async fn meter(self: Arc<Self>, ctx: TaskContext, fail: bool) -> Result<bool, TaskError> {
        if fail {
            return Err(TaskError::non_retryable("metered failure"));
        }
        self.0.typed("obs_meter", &ctx, &fail).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("obs_charge", |input| input.clone());
    probe.default_output("obs_ship", |input| input.clone());
    probe.default_output("obs_meter", |input| input.clone());
    probe.default_output("obs_flaky", |input| input.clone());
    probe.default_output("obs_wobbly", |input| input.clone());
}

#[workflow(namespace = "obs", name = "order", version = "1")]
pub struct OrderWorkflow;

#[workflow_methods]
impl OrderWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<u64> {
        let charged = ctx.call(Observed::charge, &cents).await?;
        Ok(ctx.call(Observed::ship, &charged).await?)
    }
}

#[workflow(namespace = "obs", name = "metered", version = "1")]
pub struct MeteredWorkflow;

#[workflow_methods]
impl MeteredWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, fail: bool) -> WorkflowResult<bool> {
        Ok(ctx.call(Observed::meter, &fail).await?)
    }
}

#[workflow(namespace = "obs", name = "parent", version = "1")]
pub struct ParentWorkflow;

#[workflow_methods]
impl ParentWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<u64> {
        Ok(ctx.run(OrderWorkflow::run, &cents).await?)
    }
}

#[workflow(namespace = "obs", name = "retrying", version = "1")]
pub struct RetryingWorkflow;

#[workflow_methods]
impl RetryingWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<u64> {
        Ok(ctx
            .call(Observed::flaky, &cents)
            .retry(Retry::exponential(3))
            .await?)
    }
}

#[workflow(namespace = "obs", name = "wobbly", version = "1")]
pub struct WobblyWorkflow;

#[workflow_methods]
impl WobblyWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<u64> {
        Ok(ctx
            .call(Observed::wobbly, &cents)
            .retry(Retry::exponential(3))
            .await?)
    }
}

#[workflow(namespace = "obs", name = "launcher", version = "1")]
pub struct LauncherWorkflow;

#[workflow_methods]
impl LauncherWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
        Ok(ctx.start(OrderWorkflow::run, &cents).await?)
    }
}

#[workflow(namespace = "obs", name = "looping", version = "1")]
pub struct LoopingWorkflow;

#[workflow_methods]
impl LoopingWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, round: u64) -> WorkflowResult<u64> {
        ctx.call(Observed::ship, &round).await?;
        if round == 0 {
            return ctx.continue_as_new(round + 1);
        }
        Ok(round)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<OrderWorkflow>("obs.order").input(json!(250)),
        Fixture::of::<MeteredWorkflow>("obs.metered").input(json!(false)),
        Fixture::of::<ParentWorkflow>("obs.parent").input(json!(250)),
        Fixture::of::<RetryingWorkflow>("obs.retrying").input(json!(250)),
        Fixture::of::<WobblyWorkflow>("obs.wobbly").input(json!(250)),
        Fixture::of::<LauncherWorkflow>("obs.launcher").input(json!(250)),
        Fixture::of::<LoopingWorkflow>("obs.looping").input(json!(0)),
    ]
}

/// A span as the recording subscriber saw it.
#[derive(Debug, Clone)]
pub struct SpanRecord {
    pub id: u64,
    pub name: String,
    pub fields: HashMap<String, String>,
    pub parent: Option<u64>,
}

/// A log record and the spans it was inside (innermost first).
#[derive(Debug, Clone)]
pub struct EventRecord {
    pub message: String,
    pub fields: HashMap<String, String>,
    pub scope: Vec<u64>,
}

/// What one scenario's subscriber recorded.
#[derive(Debug, Default)]
pub struct Traces {
    spans: Mutex<Vec<SpanRecord>>,
    events: Mutex<Vec<EventRecord>>,
}

impl Traces {
    pub fn spans(&self, name: &str) -> Vec<SpanRecord> {
        lock(&self.spans)
            .iter()
            .filter(|span| span.name == name)
            .cloned()
            .collect()
    }

    pub fn span(&self, id: u64) -> SpanRecord {
        lock(&self.spans)
            .iter()
            .find(|span| span.id == id)
            .cloned()
            .expect("a recorded span")
    }

    /// Every event message recorded, in order.
    pub fn messages(&self) -> Vec<String> {
        lock(&self.events)
            .iter()
            .map(|event| event.message.clone())
            .collect()
    }

    /// Every span, in the order they were created.
    pub fn all(&self) -> Vec<SpanRecord> {
        lock(&self.spans).clone()
    }

    /// The spans whose parent is `parent`, in the order they were created.
    pub fn children(&self, parent: u64) -> Vec<SpanRecord> {
        lock(&self.spans)
            .iter()
            .filter(|span| span.parent == Some(parent))
            .cloned()
            .collect()
    }

    /// The engine's event records (`jackdaw::event`) whose `event.name` is `name`.
    pub fn engine_events(&self, name: &str) -> Vec<EventRecord> {
        lock(&self.events)
            .iter()
            .filter(|event| event.fields.get("event.name").map(String::as_str) == Some(name))
            .cloned()
            .collect()
    }

    pub fn events(&self, message: &str) -> Vec<EventRecord> {
        lock(&self.events)
            .iter()
            .filter(|event| event.message == message)
            .cloned()
            .collect()
    }
}

struct Fields<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

struct Recording(Arc<Traces>);

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Recording {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = HashMap::new();
        attrs.record(&mut Fields(&mut fields));
        let parent = ctx
            .span(id)
            .and_then(|span| span.parent())
            .map(|parent| parent.id().into_u64());
        lock(&self.0.spans).push(SpanRecord {
            id: id.into_u64(),
            name: attrs.metadata().name().to_string(),
            fields,
            parent,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut spans = lock(&self.0.spans);
        if let Some(span) = spans.iter_mut().rev().find(|span| span.id == id.into_u64()) {
            values.record(&mut Fields(&mut span.fields));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = HashMap::new();
        event.record(&mut Fields(&mut fields));
        let scope = ctx
            .event_scope(event)
            .map(|scope| scope.map(|span| span.id().into_u64()).collect())
            .unwrap_or_default();
        lock(&self.0.events).push(EventRecord {
            message: fields.remove("message").unwrap_or_default(),
            fields,
            scope,
        });
    }
}

/// A recording dispatcher that lives as long as the test binary. Tracing caches each callsite's interest across all
/// dispatchers: without one that is always interested, a span first hit by a scenario that records nothing could be
/// cached as uninteresting while a recording scenario needs it.
pub fn keep_callsites_interested() -> tracing::Dispatch {
    tracing::Dispatch::new(tracing_subscriber::registry().with(Recording(Arc::default())))
}

/// The scenario's recording subscriber (installed on first use).
pub fn dispatch(world: &mut LibWorld) -> (tracing::Dispatch, Arc<Traces>) {
    let traces = Arc::clone(world.traces.get_or_insert_with(Arc::default));
    let subscriber = tracing_subscriber::registry().with(Recording(Arc::clone(&traces)));
    (tracing::Dispatch::new(subscriber), traces)
}

/// Starts the scenario's fixture with the recording subscriber as the current one.
async fn traced_start(world: &mut LibWorld, label: &str, input: Value) -> usize {
    let (dispatch, _) = dispatch(world);
    world.start(label, input).with_subscriber(dispatch).await
}

#[given("a tracing subscriber that records spans")]
fn recording(world: &mut LibWorld) {
    dispatch(world);
}

#[when("a workflow with two task steps runs")]
async fn two_steps(world: &mut LibWorld) {
    world.set_fixture(super::fixture("obs.order"));
    let run = traced_start(world, "it", json!(250)).await;
    assert_eq!(world.outcome(run).await.value(), &json!(250));
}

#[then(
    regex = r#"^there is a span "workflow" with fields "workflow\.id", "instance\.id" and "replay"$"#
)]
fn workflow_span(world: &mut LibWorld) {
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    let spans = traces.spans("workflow");
    assert_eq!(spans.len(), 1, "{spans:?}");
    let fields = &spans[0].fields;
    assert!(fields["workflow.id"].contains("order"), "{fields:?}");
    assert_eq!(fields["instance.id"], world.runs[world.last()].id);
    assert_eq!(fields["replay"], "false");
}

#[then(
    regex = r#"^inside it a span "step" for each step with fields "step\.name", "step\.kind" and "attempt"$"#
)]
fn step_spans(world: &mut LibWorld) {
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    let workflow = traces.spans("workflow")[0].id;
    let steps = traces.spans("step");
    let names: Vec<&str> = steps
        .iter()
        .map(|span| span.fields["step.name"].as_str())
        .collect();
    assert_eq!(names, vec!["obs.charge", "obs.ship"], "{steps:?}");
    for step in &steps {
        assert_eq!(step.parent, Some(workflow), "{step:?}");
        assert_eq!(step.fields["step.kind"], "call");
        assert_eq!(step.fields["attempt"], "1");
    }
}

#[given(regex = r#"^a task that logs "charging"(?: with "tracing::info!")?$"#)]
fn logging_task(world: &mut LibWorld) {
    dispatch(world);
    world.set_fixture(super::fixture("obs.order"));
}

#[when("it runs in a workflow")]
async fn runs_in_workflow(world: &mut LibWorld) {
    let run = traced_start(world, "it", json!(250)).await;
    world.outcome(run).await.value();
}

#[then(regex = r#"^the log record is inside the "attempt 1" span of the step span for that task$"#)]
fn log_in_span(world: &mut LibWorld) {
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    let charging = traces.events("charging");
    assert_eq!(charging.len(), 1);
    let attempt = traces.span(charging[0].scope[0]);
    assert_eq!(attempt.name, "attempt");
    assert_eq!(attempt.fields["otel.name"], "attempt 1");
    let step = traces.span(attempt.parent.expect("a parent"));
    assert_eq!(step.name, "step");
    assert_eq!(step.fields["step.name"], "obs.charge");
    let workflow = traces.span(step.parent.expect("a parent"));
    assert_eq!(workflow.name, "workflow");
}

#[when("the instance is replayed")]
async fn replayed(world: &mut LibWorld) {
    world.probe.script("obs_ship", |s| s.block = true);
    let run = traced_start(world, "it", json!(250)).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("shipping starts", || probe.running("obs_ship"))
        .await;
    world.crash().await;
    world.probe.release("obs_ship");
    world.probe.script("obs_ship", |s| s.block = false);
    let (dispatch, _) = dispatch(world);
    world.resume(run).with_subscriber(dispatch).await;
    assert_eq!(world.outcome(run).await.value(), &json!(250));
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    let replays: Vec<String> = traces
        .spans("workflow")
        .iter()
        .map(|span| span.fields["replay"].clone())
        .collect();
    assert_eq!(replays, vec!["false", "true"]);
}

#[then(regex = r#"^"charging" is logged once in total$"#)]
fn logged_once(world: &mut LibWorld) {
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    assert_eq!(traces.events("charging").len(), 1);
    assert_eq!(world.probe.count("obs_charge"), 1);
}

/// The test binary's metrics recorder. Installing it fails if anything (the library included) installed one before.
fn recorder() -> &'static Snapshotter {
    static SNAPSHOTTER: OnceLock<Snapshotter> = OnceLock::new();
    SNAPSHOTTER.get_or_init(|| {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        recorder
            .install()
            .expect("no metrics recorder was installed before the tests' own");
        snapshotter
    })
}

#[given(regex = r#"^the "metrics" feature is enabled and a metrics recorder is installed$"#)]
fn metrics_recorder(_world: &mut LibWorld) {
    recorder();
}

#[when(regex = r"^(\d+) workflows complete and (\d+) fails?$")]
async fn complete_and_fail(world: &mut LibWorld, ok: usize, failed: usize) {
    world.set_fixture(super::fixture("obs.metered"));
    let mut runs = Vec::new();
    for n in 0..ok + failed {
        runs.push(world.start(&format!("run-{n}"), json!(n >= ok)).await);
    }
    for run in runs {
        world.outcome(run).await;
    }
}

/// A metric's running total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Total {
    Counter(u64),
    Samples(usize),
    Other,
}

/// The metric `name` among the metered workflow's (other scenarios report to the same recorder). Snapshots drain the
/// recorder, so they are added up here.
fn metric(name: &str, label: (&str, &str)) -> Vec<Total> {
    metric_with(name, &[label])
}

/// Like [`metric`], matching every label in `labels` (by value: the workflow and step labels contain the name).
fn metric_with(name: &str, labels: &[(&str, &str)]) -> Vec<Total> {
    static SEEN: Mutex<Vec<(CompositeKey, Total)>> = Mutex::new(Vec::new());
    let mut seen = lock(&SEEN);
    for (key, _, _, value) in recorder().snapshot().into_vec() {
        let more = match value {
            DebugValue::Counter(n) => Total::Counter(n),
            DebugValue::Histogram(samples) => Total::Samples(samples.len()),
            DebugValue::Gauge(_) => Total::Other,
        };
        match seen.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, total)) => {
                *total = match (*total, more) {
                    (Total::Counter(a), Total::Counter(b)) => Total::Counter(a + b),
                    (Total::Samples(a), Total::Samples(b)) => Total::Samples(a + b),
                    (_, more) => more,
                }
            }
            None => seen.push((key, more)),
        }
    }
    seen.iter()
        .filter(|(key, _)| {
            key.key().name() == name
                && labels.iter().all(|label| {
                    key.key()
                        .labels()
                        .any(|l| l.key() == label.0 && l.value().contains(label.1))
                })
        })
        .map(|(_, total)| *total)
        .collect()
}

#[then(regex = r#"^the counter "(\w+)" is (\d+)$"#)]
fn counter_is(_world: &mut LibWorld, name: String, n: u64) {
    assert_eq!(
        metric(&name, ("workflow", "metered")),
        vec![Total::Counter(n)]
    );
}

#[then(regex = r#"^the histogram "(\w+)" has samples labelled by step name$"#)]
fn histogram_samples(_world: &mut LibWorld, name: String) {
    // One live step per workflow: three that completed and one that failed.
    let samples: usize = metric(&name, ("step", "obs.meter"))
        .iter()
        .map(|total| match total {
            Total::Samples(n) => *n,
            Total::Counter(_) | Total::Other => 0,
        })
        .sum();
    assert_eq!(samples, 4);
}

/// The samples of the step-duration histogram of `obs.meter` with `outcome`.
fn meter_samples(outcome: &str) -> Vec<Total> {
    metric_with(
        "jackdaw_step_duration_seconds",
        &[("step", "obs.meter"), ("outcome", outcome)],
    )
}

#[then(regex = r#"^its samples are labelled by outcome: (\d+) "ok" and (\d+) "fault"$"#)]
fn histogram_outcomes(_world: &mut LibWorld, ok: usize, fault: usize) {
    assert_eq!(meter_samples("ok"), vec![Total::Samples(ok)]);
    assert_eq!(meter_samples("fault"), vec![Total::Samples(fault)]);
}

#[when("a parent workflow runs a child workflow")]
async fn parent_runs_child(world: &mut LibWorld) {
    world.set_fixture(super::fixture("obs.parent"));
    let run = traced_start(world, "it", json!(250)).await;
    assert_eq!(world.outcome(run).await.value(), &json!(250));
}

#[then("the child's workflow span has the parent's step span as its parent")]
fn child_span_parent(world: &mut LibWorld) {
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    let workflows = traces.spans("workflow");
    let parent = workflows
        .iter()
        .find(|span| span.fields["workflow.id"].contains("parent"))
        .expect("the parent's span");
    let child = workflows
        .iter()
        .find(|span| span.fields["workflow.id"].contains("order"))
        .expect("the child's span");
    let step = traces.span(child.parent.expect("the child span has a parent"));
    assert_eq!(step.name, "step");
    assert_eq!(step.fields["step.kind"], "run");
    assert_eq!(step.parent, Some(parent.id));
}

#[when("I build an engine")]
fn build_engine(world: &mut LibWorld) {
    world.engine();
}

#[then("no global tracing subscriber or metrics recorder has been installed")]
fn nothing_global(_world: &mut LibWorld) {
    // A thread with no scoped subscriber of its own sees the global one, if any.
    let global = std::thread::spawn(|| {
        tracing::dispatcher::get_default(|dispatch| {
            !dispatch.is::<tracing::subscriber::NoSubscriber>()
        })
    })
    .join()
    .expect("the check ran");
    assert!(!global, "a global tracing subscriber was installed");
    // Panics if something other than the tests installed a metrics recorder.
    recorder();
}

fn traces(world: &LibWorld) -> Arc<Traces> {
    Arc::clone(world.traces.as_ref().expect("a recording subscriber"))
}

/// The step span whose `step.name` is `name`.
fn step_named(traces: &Traces, name: &str) -> SpanRecord {
    traces
        .spans("step")
        .into_iter()
        .find(|span| span.fields["step.name"] == name)
        .unwrap_or_else(|| panic!("a step span for {name}: {:?}", traces.spans("step")))
}

#[then(regex = r#"^the workflow span is named "([^"]+)"$"#)]
fn workflow_named(world: &mut LibWorld, expected: String) {
    let traces = traces(world);
    assert_eq!(traces.spans("workflow")[0].fields["otel.name"], expected);
}

#[then(regex = r#"^the step spans are named "([^"]+)" and "([^"]+)"$"#)]
fn steps_named(world: &mut LibWorld, first: String, second: String) {
    let traces = traces(world);
    let names: Vec<String> = traces
        .spans("step")
        .iter()
        .map(|span| span.fields["otel.name"].clone())
        .collect();
    assert_eq!(names, vec![first, second]);
}

#[then(regex = r#"^each step span records its position, its call site and the outcome "(\w+)"$"#)]
fn step_details(world: &mut LibWorld, outcome: String) {
    let traces = traces(world);
    for (seq, step) in traces.spans("step").iter().enumerate() {
        assert_eq!(step.fields["step.seq"], seq.to_string(), "{step:?}");
        assert_eq!(step.fields["step.ordinal"], "0", "{step:?}");
        assert!(
            step.fields["code.filepath"].ends_with("observability.rs"),
            "{step:?}"
        );
        assert!(step.fields.contains_key("code.lineno"), "{step:?}");
        assert_eq!(step.fields["step.outcome"], outcome, "{step:?}");
        assert!(!step.fields.contains_key("otel.status_code"), "{step:?}");
    }
}

#[when(regex = r"^a (metered )?task step fails once and succeeds on its retry$")]
async fn fails_once(world: &mut LibWorld, metered: String) {
    let (task, fixture) = if metered.is_empty() {
        ("obs_flaky", "obs.retrying")
    } else {
        ("obs_wobbly", "obs.wobbly")
    };
    world.probe.script(task, |s| s.fail_first = 1);
    world.set_fixture(super::fixture(fixture));
    let run = traced_start(world, "it", json!(250)).await;
    assert_eq!(world.outcome(run).await.value(), &json!(250));
}

#[then(
    regex = r#"^its step span has the spans "attempt 1", "retry backoff" and "attempt 2" in that order$"#
)]
fn attempt_spans(world: &mut LibWorld) {
    let traces = traces(world);
    let step = step_named(&traces, "obs.flaky");
    // Recording the outcome (`persist journal`) comes after them.
    let names: Vec<String> = traces
        .children(step.id)
        .iter()
        .filter(|span| span.name == "attempt" || span.name == "retry.backoff")
        .map(|span| span.fields["otel.name"].clone())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert_eq!(names[0], "attempt 1");
    assert!(names[1].starts_with("retry backoff "), "{names:?}");
    assert_eq!(names[2], "attempt 2");
}

#[then(regex = r#"^the span "attempt 1" is marked as an error that may be retried$"#)]
fn first_attempt_failed(world: &mut LibWorld) {
    let traces = traces(world);
    let first = traces
        .spans("attempt")
        .into_iter()
        .find(|span| span.fields["otel.name"] == "attempt 1")
        .expect("attempt 1");
    assert_eq!(first.fields["otel.status_code"], "ERROR", "{first:?}");
    assert_eq!(first.fields["attempt.outcome"], "retryable_error");
    assert_eq!(first.fields["attempt.retryable"], "true");
    let second = traces
        .spans("attempt")
        .into_iter()
        .find(|span| span.fields["otel.name"] == "attempt 2")
        .expect("attempt 2");
    assert_eq!(second.fields["attempt.outcome"], "ok", "{second:?}");
}

#[then(regex = r#"^the step span records attempt (\d+) and the outcome "(\w+)"$"#)]
fn step_attempt(world: &mut LibWorld, attempt: u32, outcome: String) {
    let traces = traces(world);
    let step = step_named(&traces, "obs.flaky");
    assert_eq!(step.fields["attempt"], attempt.to_string());
    assert_eq!(step.fields["step.outcome"], outcome);
}

#[then(regex = r#"^the events "(\w+)" and "(\w+)" are logged in the workflow span$"#)]
fn workflow_events(world: &mut LibWorld, first: String, second: String) {
    let traces = traces(world);
    let workflow = traces.spans("workflow")[0].id;
    for name in [first, second] {
        let events = traces.engine_events(&name);
        assert_eq!(events.len(), 1, "{name}: {events:?}");
        assert_eq!(events[0].scope.first(), Some(&workflow), "{name}");
    }
}

#[then(regex = r#"^the events "(\w+)" and "(\w+)" of "([^"]+)" are logged in its step span$"#)]
fn step_events(world: &mut LibWorld, first: String, second: String, task: String) {
    let traces = traces(world);
    let step = step_named(&traces, &task);
    for name in [first, second] {
        let events: Vec<EventRecord> = traces
            .engine_events(&name)
            .into_iter()
            .filter(|event| event.fields["task.name"] == task)
            .collect();
        assert_eq!(events.len(), 1, "{name}: {events:?}");
        assert_eq!(events[0].scope.first(), Some(&step.id), "{name}");
    }
}

#[when("a workflow whose task fails runs")]
async fn failing_workflow(world: &mut LibWorld) {
    // Not `obs.metered`: the metrics scenario counts that workflow's failures.
    world.probe.script("obs_flaky", |s| {
        s.fault = Some(FaultSpec::non_retryable("a traced failure"));
    });
    world.set_fixture(super::fixture("obs.retrying"));
    let run = traced_start(world, "it", json!(250)).await;
    world.outcome(run).await;
}

#[then(regex = r#"^the step span has the outcome "(\w+)" and the status "(\w+)"$"#)]
fn failed_step(world: &mut LibWorld, outcome: String, status: String) {
    let traces = traces(world);
    let step = step_named(&traces, "obs.flaky");
    assert_eq!(step.fields["step.outcome"], outcome, "{step:?}");
    assert_eq!(step.fields["otel.status_code"], status);
    assert!(
        step.fields["otel.status_message"].contains("a traced failure"),
        "{step:?}"
    );
    assert!(step.fields.contains_key("error.type"), "{step:?}");
}

#[then(regex = r#"^the workflow span has the outcome "(\w+)" and the status "(\w+)"$"#)]
fn failed_workflow(world: &mut LibWorld, outcome: String, status: String) {
    let traces = traces(world);
    let workflow = &traces.spans("workflow")[0];
    assert_eq!(workflow.fields["workflow.outcome"], outcome, "{workflow:?}");
    assert_eq!(workflow.fields["otel.status_code"], status);
}

#[then(regex = r#"^the resumed workflow span has one span "([^"]+)" with an event for "([^"]+)"$"#)]
fn replay_span(world: &mut LibWorld, name: String, task: String) {
    let traces = traces(world);
    let resumed = traces
        .spans("workflow")
        .into_iter()
        .find(|span| span.fields["replay"] == "true")
        .expect("the resumed workflow span");
    let replays = traces.spans("replay");
    assert_eq!(replays.len(), 1, "{replays:?}");
    assert_eq!(replays[0].parent, Some(resumed.id));
    assert_eq!(replays[0].fields["otel.name"], name);
    assert_eq!(replays[0].fields["replay.steps"], "1");
    let replayed = traces.events("step replayed");
    assert_eq!(replayed.len(), 1, "{replayed:?}");
    assert_eq!(replayed[0].fields["step.name"], task);
    assert_eq!(replayed[0].scope.first(), Some(&replays[0].id));
    // The recorded TaskStarted and TaskCompleted are reported again, inside the replay span.
    let again: Vec<EventRecord> = traces
        .engine_events("TaskCompleted")
        .into_iter()
        .filter(|event| event.fields["replayed"] == "true")
        .collect();
    assert_eq!(again.len(), 1, "{again:?}");
    assert_eq!(again[0].scope.first(), Some(&replays[0].id));
}

#[then(regex = r#"^the live step "([^"]+)" is not inside it$"#)]
fn live_step_outside_replay(world: &mut LibWorld, name: String) {
    let traces = traces(world);
    let replay = traces.spans("replay")[0].id;
    for step in traces.spans("step") {
        if step.fields["step.name"] == name {
            assert_ne!(step.parent, Some(replay), "{step:?}");
        }
    }
}

#[then(
    regex = r#"^the step spans record "step\.input\.bytes" and "step\.output\.bytes" but not "step\.input" or "step\.output"$"#
)]
fn no_payloads(world: &mut LibWorld) {
    let traces = traces(world);
    for step in traces.spans("step") {
        assert_eq!(step.fields["step.input.bytes"], "3", "{step:?}");
        assert_eq!(step.fields["step.output.bytes"], "3", "{step:?}");
        assert!(!step.fields.contains_key("step.input"), "{step:?}");
        assert!(!step.fields.contains_key("step.output"), "{step:?}");
    }
}

#[given(regex = r#"^an engine built with "trace_payloads\((\d+)\)"$"#)]
fn payload_engine(world: &mut LibWorld, max: usize) {
    world
        .extras
        .push(Arc::new(move |builder| builder.trace_payloads(max)));
}

#[then(regex = r#"^the step span of "([^"]+)" records "step\.input" as "([^"]+)"$"#)]
fn payload_recorded(world: &mut LibWorld, name: String, expected: String) {
    let traces = traces(world);
    let step = step_named(&traces, &name);
    assert_eq!(step.fields["step.input"], expected, "{step:?}");
    assert_eq!(step.fields["step.output"], expected, "{step:?}");
}

#[when(regex = r#"^a workflow starts another with "ctx\.start"$"#)]
async fn starts_another(world: &mut LibWorld) {
    world.set_fixture(super::fixture("obs.launcher"));
    let run = traced_start(world, "it", json!(250)).await;
    assert!(world.outcome(run).await.value().is_string());
    let probe = Arc::clone(&world.probe);
    world
        .until("the started workflow ships", || {
            probe.completed("obs_ship") == 1
        })
        .await;
}

#[then("the started workflow's span has no parent")]
fn started_has_no_parent(world: &mut LibWorld) {
    let traces = traces(world);
    let started = traces
        .spans("workflow")
        .into_iter()
        .find(|span| span.fields["workflow.id"].contains("order"))
        .expect("the started workflow's span");
    assert_eq!(started.parent, None, "{started:?}");
}

#[then(regex = r#"^the "start" step span records the started instance's id$"#)]
fn start_records_child(world: &mut LibWorld) {
    let traces = traces(world);
    let step = traces
        .spans("step")
        .into_iter()
        .find(|span| span.fields["step.kind"] == "start")
        .expect("the start step");
    let started = traces
        .spans("workflow")
        .into_iter()
        .find(|span| span.fields["workflow.id"].contains("order"))
        .expect("the started workflow's span");
    assert_eq!(
        step.fields["child.instance.id"],
        started.fields["instance.id"]
    );
    assert_eq!(step.fields["otel.name"], "start order");
}

#[when("a workflow continues as new once")]
async fn continues_once(world: &mut LibWorld) {
    world.set_fixture(super::fixture("obs.looping"));
    let run = traced_start(world, "it", json!(0)).await;
    assert_eq!(world.outcome(run).await.value(), &json!(1));
}

#[then(regex = r"^its step spans record the runs (\d+) and (\d+)$")]
fn runs_numbered(world: &mut LibWorld, first: u32, second: u32) {
    let traces = traces(world);
    let runs: Vec<String> = traces
        .spans("step")
        .iter()
        .map(|span| span.fields["workflow.run"].clone())
        .collect();
    assert_eq!(runs, vec![first.to_string(), second.to_string()]);
    let continued = traces.engine_events("ContinuedAsNew");
    assert_eq!(continued.len(), 1, "{continued:?}");
    assert_eq!(continued[0].fields["workflow.run"], second.to_string());
    assert_eq!(
        traces.spans("workflow")[0].fields["workflow.run"],
        second.to_string()
    );
}

#[then(
    regex = r#"^the histogram "(\w+)" of "([^"]+)" has (\d+) "(\w+)" and (\d+) "(\w+)" sample$"#
)]
fn attempt_histogram(
    _world: &mut LibWorld,
    name: String,
    task: String,
    first: usize,
    first_outcome: String,
    second: usize,
    second_outcome: String,
) {
    for (n, outcome) in [(first, first_outcome), (second, second_outcome)] {
        assert_eq!(
            metric_with(&name, &[("task", &task), ("outcome", &outcome)]),
            vec![Total::Samples(n)],
            "{outcome}"
        );
    }
}

#[then(regex = r#"^the counter "(\w+)" of "([^"]+)" is (\d+)$"#)]
fn task_counter(_world: &mut LibWorld, name: String, task: String, n: u64) {
    assert_eq!(metric(&name, ("task", &task)), vec![Total::Counter(n)]);
}

const YAML_LOOP: &str = r#"
document:
  dsl: '1.0.0'
  namespace: obs
  name: yaml-loop
  version: '1.0.0'
do:
  - prepare:
      set:
        colors: [red, blue]
  - each:
      for:
        each: color
        in: '.colors'
      do:
        - paint:
            set:
              last: '${ .color }'
"#;

#[when(
    regex = r#"^a YAML workflow sets a list and loops over it with "for", running a "set" task each time$"#
)]
async fn yaml_loop(world: &mut LibWorld) {
    let (dispatch, _) = dispatch(world);
    let workflow = jackdaw::api::Workflow::<Value, Value>::from_yaml(YAML_LOOP).expect("parses");
    let engine = world.engine();
    async move {
        let handle = engine.execute(&workflow, json!({})).await.expect("starts");
        handle.await.expect("the workflow completes");
    }
    .with_subscriber(dispatch)
    .await;
}

#[then(regex = r#"^the workflow span is named "([^"]+)" and says it ran on the "(\w+)" engine$"#)]
fn yaml_workflow_span(world: &mut LibWorld, name: String, engine: String) {
    let traces = traces(world);
    let workflows: Vec<SpanRecord> = traces
        .spans("workflow")
        .into_iter()
        .filter(|span| span.fields["otel.name"] == name)
        .collect();
    assert_eq!(workflows.len(), 1, "{:?}", traces.spans("workflow"));
    assert_eq!(workflows[0].fields["workflow.engine"], engine);
    assert_eq!(workflows[0].fields["workflow.outcome"], "completed");
}

/// The YAML task spans named `otel_name`.
fn tasks_named(traces: &Traces, otel_name: &str) -> Vec<SpanRecord> {
    traces
        .spans("task")
        .into_iter()
        .filter(|span| span.fields["otel.name"] == otel_name)
        .collect()
}

#[then(regex = r#"^its task spans are "([^"]+)" at "([^"]+)" and "([^"]+)" at "([^"]+)"$"#)]
fn yaml_top_tasks(
    world: &mut LibWorld,
    first: String,
    first_at: String,
    second: String,
    second_at: String,
) {
    let traces = traces(world);
    let workflow = traces
        .spans("workflow")
        .into_iter()
        .find(|span| span.fields["workflow.engine"] == "yaml")
        .expect("the YAML workflow span");
    for (name, at) in [(first, first_at), (second, second_at)] {
        let tasks = tasks_named(&traces, &name);
        assert_eq!(tasks.len(), 1, "{name}: {:?}", traces.spans("task"));
        assert_eq!(tasks[0].fields["task.reference"], at);
        assert_eq!(tasks[0].parent, Some(workflow.id), "{name}");
        assert_eq!(tasks[0].fields["task.outcome"], "ok", "{name}");
    }
}

#[then(regex = r#"^"([^"]+)" has the spans "iteration 0" and "iteration 1"$"#)]
fn yaml_iterations(world: &mut LibWorld, task: String) {
    let traces = traces(world);
    let task = &tasks_named(&traces, &task)[0];
    let names: Vec<String> = traces
        .children(task.id)
        .iter()
        .filter(|span| span.name == "iteration")
        .map(|span| span.fields["otel.name"].clone())
        .collect();
    assert_eq!(names, vec!["iteration 0", "iteration 1"]);
}

#[then(regex = r#"^each iteration has a span "([^"]+)" at "([^"]+)"$"#)]
fn yaml_iteration_tasks(world: &mut LibWorld, name: String, at: String) {
    let traces = traces(world);
    let tasks = tasks_named(&traces, &name);
    assert_eq!(tasks.len(), 2, "{:?}", traces.spans("task"));
    let mut parents: Vec<String> = tasks
        .iter()
        .map(|task| {
            assert_eq!(task.fields["task.reference"], at);
            let parent = traces.span(task.parent.expect("a parent"));
            assert_eq!(parent.name, "iteration");
            parent.fields["otel.name"].clone()
        })
        .collect();
    parents.sort();
    assert_eq!(parents, vec!["iteration 0", "iteration 1"]);
}

#[then(regex = r#"^the event "(\w+)" of "([^"]+)" is logged in a "([^"]+)" span$"#)]
fn yaml_event_in_task(world: &mut LibWorld, event: String, task: String, span: String) {
    let traces = traces(world);
    let spans: Vec<u64> = tasks_named(&traces, &span).iter().map(|s| s.id).collect();
    let events: Vec<EventRecord> = traces
        .engine_events(&event)
        .into_iter()
        .filter(|record| record.fields.get("task.name") == Some(&task))
        .collect();
    assert_eq!(events.len(), 2, "{events:?}");
    for record in events {
        assert!(spans.contains(&record.scope[0]), "{record:?}");
    }
}

#[then(
    regex = r#"^the histogram "jackdaw_workflow_duration_seconds" has (\d+) "completed" and (\d+) "failed" sample$"#
)]
fn workflow_durations(_world: &mut LibWorld, completed: usize, failed: usize) {
    for (n, outcome) in [(completed, "completed"), (failed, "failed")] {
        assert_eq!(
            metric_with(
                "jackdaw_workflow_duration_seconds",
                &[("workflow", "metered"), ("outcome", outcome)]
            ),
            vec![Total::Samples(n)],
            "{outcome}"
        );
    }
}

#[then(regex = r#"^the histogram "(\w+)" has samples for "(\w+)"$"#)]
fn store_samples(_world: &mut LibWorld, name: String, operation: String) {
    // Every scenario writes journals, so only that there are some is certain.
    let totals = metric(&name, ("operation", &operation));
    assert!(
        totals
            .iter()
            .any(|total| matches!(total, Total::Samples(n) if *n > 0)),
        "{totals:?}"
    );
}
