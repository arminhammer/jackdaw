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

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<OrderWorkflow>("obs.order").input(json!(250)),
        Fixture::of::<MeteredWorkflow>("obs.metered").input(json!(false)),
        Fixture::of::<ParentWorkflow>("obs.parent").input(json!(250)),
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
fn dispatch(world: &mut LibWorld) -> (tracing::Dispatch, Arc<Traces>) {
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

#[then("the log record is inside the step span for that task")]
fn log_in_span(world: &mut LibWorld) {
    let traces = Arc::clone(world.traces.as_ref().expect("a recording subscriber"));
    let charging = traces.events("charging");
    assert_eq!(charging.len(), 1);
    let step = traces.span(charging[0].scope[0]);
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
                && key
                    .key()
                    .labels()
                    .any(|l| l.key() == label.0 && l.value().contains(label.1))
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
    assert_eq!(
        metric(&name, ("step", "obs.meter")),
        vec![Total::Samples(4)]
    );
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
