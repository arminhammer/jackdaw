//! durability/*.feature: durable timers, crash recovery, replay and versioning, the journal.
use super::support::*;
use super::world::{Fixture, LibWorld};
use async_trait::async_trait;
use cucumber::{given, then, when};
use jackdaw::api::{ExecuteOptions, PayloadCodec};
use jackdaw::persistence::{JournalRecord, PersistenceProvider};
use jackdaw::prelude::*;
use jackdaw::providers::persistence::InMemoryPersistence;
use jackdaw::workflow::{WorkflowCheckpoint, WorkflowEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------------------------
// Tasks and workflows
// ---------------------------------------------------------------------------------------------------------------

pub struct Durable(pub Arc<Probe>);

#[tasks]
impl Durable {
    #[task(name = "dur.a")]
    pub async fn a(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<String, TaskError> {
        self.0.typed("a", &ctx, &()).await
    }
    #[task(name = "dur.b")]
    pub async fn b(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<String, TaskError> {
        self.0.typed("b", &ctx, &()).await
    }
    #[task(name = "dur.c")]
    pub async fn c(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<String, TaskError> {
        self.0.typed("c", &ctx, &()).await
    }
    #[task(name = "dur.first")]
    pub async fn first(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("dur_first", &ctx, &()).await
    }
    #[task(name = "dur.second")]
    pub async fn second(self: Arc<Self>, ctx: TaskContext, n: u32) -> Result<u32, TaskError> {
        self.0.typed("dur_second", &ctx, &n).await
    }
    #[task(name = "dur.echo")]
    pub async fn echo(self: Arc<Self>, ctx: TaskContext, value: Value) -> Result<Value, TaskError> {
        self.0.typed("echo", &ctx, &value).await
    }
    #[task(name = "dur.label")]
    pub async fn label(
        self: Arc<Self>,
        ctx: TaskContext,
        carrier: String,
    ) -> Result<String, TaskError> {
        self.0.typed("label", &ctx, &carrier).await
    }
}

pub fn defaults(probe: &Probe) {
    for name in ["a", "b", "c"] {
        let name = name.to_string();
        probe.default_output(&name.clone(), move |_| json!(name));
    }
    probe.default_output("dur_first", |_| json!(null));
    probe.default_output("dur_second", |n| n.clone());
    probe.default_output("echo", |v| v.clone());
    probe.default_output("label", |carrier| {
        json!(format!("label via {}", carrier.as_str().unwrap_or("?")))
    });
}

#[workflow(namespace = "dur", name = "sleep", version = "1")]
pub struct SleepThen;

#[workflow_methods]
impl SleepThen {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, seconds: u64) -> WorkflowResult<String> {
        ctx.sleep(seconds.secs()).await?;
        Ok("done".into())
    }
}

#[workflow(namespace = "dur", name = "abc", version = "1")]
pub struct Abc;

#[workflow_methods]
impl Abc {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<String>> {
        let a = ctx.call(Durable::a, &()).await?;
        let b = ctx.call(Durable::b, &()).await?;
        let c = ctx.call(Durable::c, &()).await?;
        Ok(vec![a, b, c])
    }
}

/// The same identity as `Abc`, with `c` where `b` was: incompatible with `Abc` histories past the first step.
pub mod changed {
    use super::*;

    #[workflow(namespace = "dur", name = "abc", version = "1")]
    pub struct Abc;

    #[workflow_methods]
    impl Abc {
        #[run]
        async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<String>> {
            let a = ctx.call(Durable::a, &()).await?;
            let c = ctx.call(Durable::c, &()).await?;
            Ok(vec![a, c])
        }
    }
}

macro_rules! versioned {
    ($module:ident, $namespace:literal, $name:literal, $version:literal, $tag:literal) => {
        pub mod $module {
            use super::*;

            #[workflow(namespace = $namespace, name = $name, version = $version)]
            pub struct W;

            #[workflow_methods]
            impl W {
                #[run]
                async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
                    ctx.call(Durable::b, &()).await?;
                    Ok($tag.to_string())
                }
            }
        }
    };
}

versioned!(order_v1, "shop", "order", "1.0.0", "v1");
versioned!(order_v2, "shop", "order", "2.0.0", "v2");
versioned!(order_v09, "shop", "order", "0.9.0", "v0.9");
versioned!(refund_v1, "shop", "refund", "1.0.0", "refund");

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum SecondStep {
    Task,
    Retrying,
    Sleep,
    Event,
    Condition,
    Child,
    ForEach,
    Race,
    Yaml,
}

#[workflow(namespace = "dur", name = "child", version = "1")]
pub struct Child;

#[workflow_methods]
impl Child {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, n: u32) -> WorkflowResult<u32> {
        Ok(ctx.call(Durable::second, &n).await?)
    }
}

const ECHO_YAML: &str = r#"
document:
  dsl: '1.0.3'
  namespace: dur
  name: echo
  version: '1.0.0'
do:
  - echo:
      set:
        echoed: '${ .n }'
"#;

#[workflow(namespace = "dur", name = "two-steps", version = "1")]
pub struct TwoSteps {
    go: bool,
}

#[workflow_methods]
impl TwoSteps {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { go: false }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, second: SecondStep) -> WorkflowResult<Value> {
        ctx.call(Durable::first, &()).await?;
        Ok(match second {
            SecondStep::Task => json!(ctx.call(Durable::second, &1).await?),
            SecondStep::Retrying => json!(
                ctx.call(Durable::second, &1)
                    .retry(Retry::exponential(5))
                    .await?
            ),
            SecondStep::Sleep => {
                ctx.sleep(1.hours()).await?;
                json!("slept")
            }
            SecondStep::Event => json!(ctx.wait_for_event::<u32>("paid").await?),
            SecondStep::Condition => {
                ctx.wait_condition(|w| w.go).await?;
                json!("went")
            }
            SecondStep::Child => json!(ctx.run(Child::run, &7).await?),
            SecondStep::ForEach => json!(
                ctx.for_each(&[1, 2, 3, 4, 5], Durable::second)
                    .concurrency(2)
                    .await?
            ),
            SecondStep::Race => {
                let winner = jackdaw::select! {
                    paid = ctx.wait_for_event::<u32>("paid") => json!(paid?),
                    _ = ctx.sleep(24.hours()) => json!("expired"),
                };
                winner
            }
            SecondStep::Yaml => {
                let workflow =
                    Workflow::<Value, Value>::from_yaml(ECHO_YAML).map_err(StepError::Engine)?;
                ctx.run(&workflow, &json!({ "n": 9 })).await?
            }
        })
    }

    #[signal]
    fn go(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.go = true;
    }
}

#[workflow(namespace = "dur", name = "parent", version = "1")]
pub struct Parent;

#[workflow_methods]
impl Parent {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, n: u32) -> WorkflowResult<u32> {
        Ok(ctx.run(Child::run, &n).await? + 1)
    }
}

#[workflow(namespace = "dur", name = "carrier", version = "1")]
pub struct Carrier;

#[workflow_methods]
impl Carrier {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        ctx.call(Durable::b, &()).await?;
        Ok(ctx.call(Durable::label, &"old-carrier".to_string()).await?)
    }
}

pub mod carrier_patched {
    use super::*;

    #[workflow(namespace = "dur", name = "carrier", version = "1")]
    pub struct Carrier;

    #[workflow_methods]
    impl Carrier {
        #[run]
        async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
            ctx.call(Durable::b, &()).await?;
            let carrier = if ctx.patched("use-new-carrier") {
                "new-carrier"
            } else {
                "old-carrier"
            };
            Ok(ctx.call(Durable::label, &carrier.to_string()).await?)
        }
    }
}

#[workflow(namespace = "dur", name = "echo", version = "1")]
pub struct Echo;

#[workflow_methods]
impl Echo {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, value: Value) -> WorkflowResult<Value> {
        Ok(ctx.call(Durable::echo, &value).await?)
    }
}

#[workflow(namespace = "dur", name = "wait-one", version = "1")]
pub struct WaitOne;

#[workflow_methods]
impl WaitOne {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        ctx.call(Durable::first, &()).await?;
        Ok(ctx.wait_for_event::<u32>("go").await?)
    }
}

#[workflow(namespace = "dur", name = "legacy", version = "1")]
pub struct Legacy;

#[workflow_methods]
impl Legacy {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        let one = ctx.call(Durable::second, &1).await?;
        let two = ctx.call(Durable::second, &2).await?;
        Ok(one + two)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<SleepThen>("dur.sleep"),
        Fixture::of::<Abc>("dur.abc"),
        Fixture::of::<order_v1::W>("dur.order-v1"),
        Fixture::of::<order_v2::W>("dur.order-v2"),
        Fixture::of::<refund_v1::W>("dur.refund-v1"),
        Fixture::of::<TwoSteps>("dur.two-steps"),
        Fixture::of::<Child>("dur.child").input(json!(7)),
        Fixture::of::<Parent>("dur.parent").input(json!(7)),
        Fixture::of::<Carrier>("dur.carrier"),
        Fixture::of::<Echo>("dur.echo"),
        Fixture::of::<WaitOne>("dur.wait-one"),
        Fixture::of::<Legacy>("dur.legacy"),
    ]
}

pub fn variants() -> Vec<Fixture> {
    vec![
        Fixture::of::<changed::Abc>("dur.abc.changed"),
        Fixture::of::<order_v09::W>("dur.order-v0.9"),
        Fixture::of::<carrier_patched::Carrier>("dur.carrier.patched"),
    ]
}

// ---------------------------------------------------------------------------------------------------------------
// Test stores
// ---------------------------------------------------------------------------------------------------------------

/// An in-memory store that records every write and can be told to fail.
#[derive(Debug, Default)]
pub struct TestStore {
    inner: InMemoryPersistence,
    /// (kind, bytes) of every write.
    pub writes: Mutex<Vec<(&'static str, usize)>>,
    /// Every byte written, for "the store does not contain .." checks.
    pub raw: Mutex<Vec<String>>,
    appends: AtomicUsize,
    pub fail_appends_after: Option<usize>,
    pub fail_events: bool,
}

impl TestStore {
    fn record(&self, kind: &'static str, value: &impl Serialize) {
        let text = serde_json::to_string(value).unwrap_or_default();
        lock(&self.writes).push((kind, text.len()));
        lock(&self.raw).push(text);
    }

    fn fail(what: &str) -> jackdaw::persistence::Error {
        jackdaw::persistence::Error::Database {
            message: format!("the test store refuses {what}"),
        }
    }

    pub fn sizes(&self, kind: &str) -> Vec<usize> {
        lock(&self.writes)
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, n)| *n)
            .collect()
    }

    pub fn contains(&self, needle: &str) -> bool {
        lock(&self.raw).iter().any(|text| text.contains(needle))
    }
}

#[async_trait]
impl PersistenceProvider for TestStore {
    async fn save_event(&self, event: WorkflowEvent) -> jackdaw::persistence::Result<()> {
        if self.fail_events {
            return Err(Self::fail("event writes"));
        }
        self.record("event", &event);
        self.inner.save_event(event).await
    }

    async fn get_events(
        &self,
        instance_id: &str,
    ) -> jackdaw::persistence::Result<Vec<WorkflowEvent>> {
        self.inner.get_events(instance_id).await
    }

    async fn save_checkpoint(
        &self,
        checkpoint: WorkflowCheckpoint,
    ) -> jackdaw::persistence::Result<()> {
        self.record("checkpoint", &checkpoint);
        self.inner.save_checkpoint(checkpoint).await
    }

    async fn get_checkpoint(
        &self,
        instance_id: &str,
    ) -> jackdaw::persistence::Result<Option<WorkflowCheckpoint>> {
        self.inner.get_checkpoint(instance_id).await
    }

    async fn append_journal(
        &self,
        instance_id: &str,
        records: Vec<JournalRecord>,
    ) -> jackdaw::persistence::Result<()> {
        let done = self.appends.fetch_add(1, Ordering::SeqCst);
        if self.fail_appends_after.is_some_and(|limit| done >= limit) {
            return Err(Self::fail("journal writes"));
        }
        self.record("append", &records);
        self.inner.append_journal(instance_id, records).await
    }

    async fn load_journal(
        &self,
        instance_id: &str,
    ) -> jackdaw::persistence::Result<Vec<JournalRecord>> {
        self.inner.load_journal(instance_id).await
    }

    async fn prune_journal(
        &self,
        instance_id: &str,
        before: u64,
    ) -> jackdaw::persistence::Result<()> {
        self.inner.prune_journal(instance_id, before).await
    }

    async fn list_instances(&self) -> jackdaw::persistence::Result<Option<Vec<String>>> {
        self.inner.list_instances().await
    }

    async fn delete_instance(&self, instance_id: &str) -> jackdaw::persistence::Result<()> {
        self.inner.delete_instance(instance_id).await
    }
}

fn use_store(world: &mut LibWorld, store: TestStore) -> Arc<TestStore> {
    let store = Arc::new(store);
    world.persistence = Some(Arc::clone(&store) as Arc<dyn PersistenceProvider>);
    world.test_store = Some(Arc::clone(&store));
    world.durable = true;
    store
}

fn store_of(world: &LibWorld) -> Arc<TestStore> {
    Arc::clone(
        world
            .test_store
            .as_ref()
            .expect("the scenario's test store"),
    )
}

/// "Encryption" for tests: XOR with a key. Enough to keep plain text out of the store.
#[derive(Debug, Clone)]
pub struct XorCodec(pub u8);

impl PayloadCodec for XorCodec {
    fn name(&self) -> &str {
        "test-xor"
    }
    fn encode(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        Ok(plain.iter().map(|b| b ^ self.0).collect())
    }
    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>, String> {
        Ok(encoded.iter().map(|b| b ^ self.0).collect())
    }
}

/// Run-length encoding: shrinks the repetitive payloads the tests use.
#[derive(Debug, Clone)]
pub struct RleCodec;

impl PayloadCodec for RleCodec {
    fn name(&self) -> &str {
        "test-rle"
    }
    fn encode(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut iter = plain.iter().peekable();
        while let Some(&byte) = iter.next() {
            let mut run: u32 = 1;
            while iter.peek() == Some(&&byte) && run < u32::MAX {
                iter.next();
                run += 1;
            }
            out.extend_from_slice(&run.to_le_bytes());
            out.push(byte);
        }
        Ok(out)
    }
    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        for chunk in encoded.chunks(5) {
            let [a, b, c, d, byte] = chunk else {
                return Err("truncated".into());
            };
            let run = u32::from_le_bytes([*a, *b, *c, *d]);
            out.extend(std::iter::repeat_n(*byte, run as usize));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// durable_timers.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(
    regex = r#"^a workflow that calls "ctx\.sleep\((\d+)\.(secs|minutes|hours)\(\)\)"(?: and then returns "done")?$"#
)]
fn sleep_then(world: &mut LibWorld, n: u64, unit: String) {
    let seconds = match unit.as_str() {
        "secs" => n,
        "minutes" => n * 60,
        _ => n * 3600,
    };
    world.set_fixture(super::fixture("dur.sleep").input(json!(seconds)));
}

#[when(regex = r"^the clock advances (\d+ \w+)$")]
async fn clock_advances(world: &mut LibWorld, span: String) {
    world.pass(duration(&span)).await;
}

#[when("the engine is dropped")]
async fn engine_dropped(world: &mut LibWorld) {
    world.quiesce().await;
    world.crash().await;
}

#[when("a new engine on the same store resumes the instance")]
async fn new_engine_resumes(world: &mut LibWorld) {
    let last = world.last();
    world.resume(last).await;
}

#[then("the workflow is still sleeping")]
async fn still_sleeping(world: &mut LibWorld) {
    world.quiesce().await;
    let last = world.last();
    assert!(!world.ended(last));
    let id = world.runs[last].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert_eq!(details.timers.len(), 1, "{details:?}");
}

#[then(regex = r#"^the workflow completes with "done" without waiting$"#)]
async fn done_without_waiting(world: &mut LibWorld) {
    let before = world.clock.now();
    let handle = world.handle(world.last());
    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("the instance ended without the clock moving");
    assert_eq!(result.expect("completes"), json!("done"));
    assert_eq!(world.clock.now(), before);
}

#[when(regex = r#"^I execute it at "(.+)"$"#)]
async fn execute_at(world: &mut LibWorld, at: String) {
    let at: chrono::DateTime<chrono::Utc> = at.parse().expect("a time");
    world.clock.advance_to(at);
    assert_eq!(
        world.clock.now(),
        at,
        "the clock starts at 2026-01-01T00:00:00Z"
    );
    let input = world.fixture().input;
    world.start("it", input).await;
    world.quiesce().await;
}

#[then(regex = r#"^the journal records the sleep step with fire time "(.+)"$"#)]
async fn records_fire_time(world: &mut LibWorld, at: String) {
    let at: chrono::DateTime<chrono::Utc> = at.parse().expect("a time");
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert!(
        details
            .timers
            .iter()
            .any(|timer| timer.key.starts_with("sleep:") && timer.fires_at == at),
        "{details:?}"
    );
}

#[given(regex = r"^a completed instance that slept for 1 hour$")]
async fn slept_an_hour(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("dur.sleep"), json!(3600))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[when(regex = r#"^I call "Engine::replay_all"$"#)]
async fn call_replay_all(world: &mut LibWorld) {
    world.notes.insert("now".into(), json!(world.clock.now()));
    super::common::replay_all(world).await;
}

#[then("the replay finishes without advancing the clock")]
fn replay_no_clock(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    assert_eq!(json!(world.clock.now()), world.notes["now"]);
    assert_eq!(json!(world.clock.sleeps_started()), world.notes["sleeps"]);
}

#[given(regex = r#"^a "TestEngine" with a workflow that sleeps 30 days$"#)]
fn test_engine_thirty(world: &mut LibWorld) {
    world.notes.insert("test-engine".into(), json!(true));
}

#[then("it completes immediately in wall-clock time")]
fn immediately(world: &mut LibWorld) {
    assert!(world.notes["wall"].as_f64().expect("timed") < 1.0);
}

#[then(regex = r#"^"TestEngine::clock\(\)\.elapsed\(\)" is 30 days$"#)]
fn elapsed_thirty(world: &mut LibWorld) {
    assert_eq!(world.notes["virtual_days"], json!(30));
}

// ---------------------------------------------------------------------------------------------------------------
// crash_recovery.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(regex = r#"^a workflow with tasks "A", "B" and "C" that count their invocations$"#)]
fn abc(world: &mut LibWorld) {
    world.set_fixture(super::fixture("dur.abc"));
}

#[given(regex = r#"^"(\w+)" blocks until it is released$"#)]
fn blocks(world: &mut LibWorld, task: String) {
    world.probe.script(&task, |s| s.block = true);
}

#[when("I execute the workflow")]
async fn execute_workflow(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
}

#[when(regex = r#"^"(\w+)" has completed$"#)]
async fn has_completed(world: &mut LibWorld, task: String) {
    let probe = Arc::clone(&world.probe);
    world
        .until("the task completed", || probe.completed(&task) >= 1)
        .await;
}

#[when(regex = r#"^the engine is dropped while "(\w+)" is running$"#)]
async fn dropped_while_running(world: &mut LibWorld, task: String) {
    let probe = Arc::clone(&world.probe);
    world.until("the task runs", || probe.running(&task)).await;
    world.crash().await;
}

#[when(regex = r#"^a new engine resumes the instance with "Engine::resume::<W>\(id\)"$"#)]
async fn resume_with(world: &mut LibWorld) {
    let last = world.last();
    world.resume(last).await;
}

#[when(regex = r#"^"(\w+)" is released$"#)]
fn released(world: &mut LibWorld, task: String) {
    world.probe.release(&task);
}

#[then(regex = r#"^the "(\w+)" event was emitted$"#)]
async fn event_emitted(world: &mut LibWorld, name: String) {
    let last = world.last();
    world.outcome(last).await;
    let mut events = world.handle(last).events();
    let mut seen = Vec::new();
    while let Some(event) = events.next().await {
        seen.push(event.name());
    }
    assert!(seen.contains(&name.as_str()), "{seen:?}");
}

#[given(regex = r#"^an instance of workflow "shop/order@1\.0\.0" in the store$"#)]
async fn order_instance(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.order-v1"), json!(null))
        .await;
    world.quiesce().await;
}

#[when(regex = r#"^a new engine calls "Engine::resume::<RefundWorkflow>\(id\)"$"#)]
async fn resume_as_refund(world: &mut LibWorld) {
    world.crash().await;
    let id = world.runs[world.last()].id.clone();
    let error = (super::fixture("dur.refund-v1").resume)(world.engine(), id)
        .await
        .expect_err("a refund workflow cannot resume an order");
    world
        .notes
        .insert("error".into(), json!(format!("{error:?}")));
}

#[then(regex = r#"^it fails with "Error::Invalid" saying the instance belongs to "(.+)"$"#)]
fn invalid_belongs(world: &mut LibWorld, workflow: String) {
    let error = world.notes["error"].as_str().expect("an error").to_string();
    assert!(error.starts_with("Invalid("), "{error}");
    assert!(error.contains(&workflow), "{error}");
}

#[given(
    regex = r#"^three instances in the store with statuses "Running", "Running" and "Completed"$"#
)]
async fn three_instances(world: &mut LibWorld) {
    world
        .start_fixture("done", super::fixture("dur.wait-one"), json!(null))
        .await;
    let done = world.run_index("done");
    world.handle(done).send_event("go", 1).await.expect("send");
    world.outcome(done).await.value();
    world
        .start_fixture("r1", super::fixture("dur.wait-one"), json!(null))
        .await;
    world
        .start_fixture("r2", super::fixture("dur.wait-one"), json!(null))
        .await;
    world.quiesce().await;
    world.mark = Some(world.probe.count("dur_first"));
}

#[when(regex = r#"^a new engine is built on the store and "Engine::recover\(\)" is called$"#)]
async fn recover_called(world: &mut LibWorld) {
    world.crash().await;
    world.recover().await;
}

#[then("the two running instances are resumed")]
fn two_resumed(world: &mut LibWorld) {
    let recovery = world.recovery.clone().expect("recovered");
    let mut resumed = recovery.resumed.clone();
    resumed.sort();
    let mut expected = vec![
        world.runs[world.run_index("r1")].id.clone(),
        world.runs[world.run_index("r2")].id.clone(),
    ];
    expected.sort();
    assert_eq!(resumed, expected);
}

#[then("the completed instance is not touched")]
fn completed_untouched(world: &mut LibWorld) {
    let done = world.runs[world.run_index("done")].id.clone();
    assert!(
        !world
            .recovery
            .as_ref()
            .expect("recovered")
            .resumed
            .contains(&done)
    );
    assert_eq!(Some(world.probe.count("dur_first")), world.mark);
}

#[then(regex = r#"^"recover" returns the ids it resumed$"#)]
async fn recover_ids(world: &mut LibWorld) {
    for label in ["r1", "r2"] {
        let index = world.run_index(label);
        world.handle(index).send_event("go", 2).await.expect("send");
        assert_eq!(world.outcome(index).await.value(), &json!(2));
    }
}

#[given(regex = r#"^a running instance of workflow "shop/order@0\.9\.0" in the store$"#)]
async fn old_version_running(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world.only = Some(
        super::catalog()
            .into_iter()
            .chain([super::variant("dur.order-v0.9")])
            .map(|f| f.register)
            .collect(),
    );
    world
        .start_fixture("old", super::variant("dur.order-v0.9"), json!(null))
        .await;
    world
        .start_fixture("current", super::fixture("dur.order-v1"), json!(null))
        .await;
    world.quiesce().await;
}

#[given(regex = r#"^the new engine only registers "shop/order@1\.0\.0"$"#)]
fn only_v1(world: &mut LibWorld) {
    world.notes.insert("drop-v0.9".into(), json!(true));
}

#[when(regex = r#"^"Engine::recover\(\)" is called$"#)]
async fn recover_is_called(world: &mut LibWorld) {
    world.crash().await;
    if world.notes.contains_key("drop-v0.9") {
        world.only = None;
    }
    world.probe.release_all();
    world.recover().await;
}

#[then("the instance is reported as unrecoverable with its workflow id and version")]
fn unrecoverable(world: &mut LibWorld) {
    let recovery = world.recovery.clone().expect("recovered");
    let old = world.runs[world.run_index("old")].id.clone();
    let report = recovery
        .unrecoverable
        .iter()
        .find(|u| u.id == old)
        .expect("reported");
    assert_eq!(report.workflow, "shop/order@0.9.0");
}

#[then(regex = r#"^it stays "Running" in the store$"#)]
async fn stays_running(world: &mut LibWorld) {
    let old = world.runs[world.run_index("old")].id.clone();
    let status = world.engine().status(&old).await.expect("status");
    assert_eq!(format!("{status:?}"), "Running");
}

#[then("the other instances are still recovered")]
async fn others_recovered(world: &mut LibWorld) {
    let current = world.run_index("current");
    assert!(
        world
            .recovery
            .as_ref()
            .expect("recovered")
            .resumed
            .contains(&world.runs[current].id)
    );
    assert_eq!(world.outcome(current).await.value(), &json!("v1"));
}

fn second_step(text: &str) -> SecondStep {
    match text {
        "a task call" => SecondStep::Task,
        "a task call that is retrying after a failure" => SecondStep::Retrying,
        "ctx.sleep(1.hours())" => SecondStep::Sleep,
        "ctx.wait_for_event(\"paid\")" => SecondStep::Event,
        "ctx.wait_condition on a signal-driven flag" => SecondStep::Condition,
        "a child workflow" => SecondStep::Child,
        "ctx.for_each over 5 items with concurrency 2" => SecondStep::ForEach,
        "race of a sleep and an event" => SecondStep::Race,
        "ctx.run_definition of a YAML workflow" => SecondStep::Yaml,
        other => panic!("unknown second step `{other}`"),
    }
}

#[given(regex = r"^a workflow whose second step is (.+)$")]
fn second_is(world: &mut LibWorld, text: String) {
    let kind = second_step(&text);
    world.notes.insert("dur_second".into(), json!(kind));
    match kind {
        SecondStep::Task | SecondStep::Child => {
            world.probe.script("dur_second", |s| s.block = true)
        }
        SecondStep::Retrying => world.probe.script("dur_second", |s| {
            s.fail_first = 1;
            s.block_on = None;
        }),
        SecondStep::ForEach => world
            .probe
            .script("dur_second", |s| s.block_on = Some(json!(3))),
        SecondStep::Sleep
        | SecondStep::Event
        | SecondStep::Condition
        | SecondStep::Race
        | SecondStep::Yaml => {}
    }
    world.set_fixture(super::fixture("dur.two-steps").input(json!(kind)));
}

fn second_kind(world: &LibWorld) -> SecondStep {
    serde_json::from_value(world.notes["dur_second"].clone()).expect("a second step")
}

#[given("the engine is dropped while the second step is in flight")]
async fn dropped_in_second(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
    let probe = Arc::clone(&world.probe);
    match second_kind(world) {
        SecondStep::Task | SecondStep::Child | SecondStep::ForEach => {
            world
                .until("the second step runs", || probe.running("dur_second"))
                .await;
        }
        SecondStep::Retrying => {
            world
                .until_frozen("the first attempt failed", || {
                    probe.count("dur_second") >= 1
                })
                .await;
        }
        SecondStep::Sleep
        | SecondStep::Event
        | SecondStep::Condition
        | SecondStep::Race
        | SecondStep::Yaml => {
            world
                .until_frozen("the first step completed", || {
                    probe.completed("dur_first") >= 1
                })
                .await;
        }
    }
    world.crash().await;
    world.probe.release_all();
}

#[when("a new engine recovers the instance")]
async fn recovers_the_instance(world: &mut LibWorld) {
    world.recover().await;
    let last = world.last();
    match second_kind(world) {
        SecondStep::Event | SecondStep::Race => {
            world
                .handle(last)
                .send_event("paid", 5)
                .await
                .expect("send");
        }
        SecondStep::Condition => {
            world
                .handle(last)
                .signal(TwoSteps::go, ())
                .await
                .expect("signal");
        }
        _ => {}
    }
}

#[then(regex = r"^the first step's task ran 1 time$")]
fn first_once(world: &mut LibWorld) {
    assert_eq!(world.probe.count("dur_first"), 1);
}

#[then("the second step ran to completion exactly once")]
async fn second_once(world: &mut LibWorld) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    match second_kind(world) {
        SecondStep::Task | SecondStep::Child => {
            assert_eq!(
                world.probe.completed("dur_second"),
                1,
                "{:?}",
                world.probe.calls_of("dur_second")
            );
        }
        SecondStep::Retrying => {
            // One failed attempt before the crash, one successful attempt after it.
            assert_eq!(value, json!(1));
            assert_eq!(
                world.probe.count("dur_second"),
                2,
                "{:?}",
                world.probe.calls_of("dur_second")
            );
        }
        SecondStep::ForEach => {
            assert_eq!(value, json!([1, 2, 3, 4, 5]));
            for n in 1..=5 {
                let done = world
                    .probe
                    .calls_of("dur_second")
                    .iter()
                    .filter(|c| c.input == json!(n) && !c.cancelled)
                    .count();
                assert_eq!(done, 1, "item {n}");
            }
        }
        SecondStep::Sleep => assert_eq!(value, json!("slept")),
        SecondStep::Event | SecondStep::Race => assert_eq!(value, json!(5)),
        SecondStep::Condition => assert_eq!(value, json!("went")),
        SecondStep::Yaml => assert_eq!(value["echoed"], json!(9)),
    }
}

#[given("a parent that is waiting on a running child")]
async fn parent_waiting(world: &mut LibWorld) {
    world.probe.script("dur_second", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.parent"), json!(7))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the child runs", || probe.running("dur_second"))
        .await;
}

#[given("the engine is dropped")]
async fn given_engine_dropped(world: &mut LibWorld) {
    world.crash().await;
    world.probe.release_all();
}

#[when("a new engine recovers")]
async fn a_new_engine_recovers(world: &mut LibWorld) {
    world.recover().await;
}

#[then("the child resumes")]
async fn child_resumes(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.completed("dur_second"), 1);
}

#[then("the parent resumes and receives the child's result")]
async fn parent_receives(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(8));
}

#[then("the child is not started a second time")]
fn child_once(world: &mut LibWorld) {
    let instances: std::collections::HashSet<String> = world
        .probe
        .calls_of("dur_second")
        .into_iter()
        .map(|c| c.instance)
        .collect();
    assert_eq!(instances.len(), 1);
}

#[given("a task whose completion is persisted")]
async fn completion_persisted(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("a completed", || {
            probe.completed("a") >= 1 && probe.running("b")
        })
        .await;
}

#[given("the engine is dropped right after the persist")]
async fn dropped_after_persist(world: &mut LibWorld) {
    world.crash().await;
    world.probe.release_all();
}

#[then("the task does not run again")]
async fn not_again(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.count("a"), 1);
}

#[given("a task that records its idempotency key and was running when the engine was dropped")]
async fn key_recorded(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world.until("b runs", || probe.running("b")).await;
    world.crash().await;
    world.probe.release_all();
}

#[then("the task runs again")]
async fn runs_again(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.count("b"), 2);
}

#[then("it receives the same idempotency key as the interrupted attempt")]
fn same_key(world: &mut LibWorld) {
    let calls = world.probe.calls_of("b");
    assert_eq!(calls[0].key, calls[1].key);
}

// ---------------------------------------------------------------------------------------------------------------
// replay_and_versioning.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(regex = r"^a store with (\d+) completed and (\d+) running instances of a workflow$")]
async fn completed_and_running(world: &mut LibWorld, completed: usize, running: usize) {
    for i in 0..completed {
        let label = format!("done{i}");
        world
            .start_fixture(&label, super::fixture("dur.wait-one"), json!(null))
            .await;
        let index = world.run_index(&label);
        world.handle(index).send_event("go", 1).await.expect("send");
        world.outcome(index).await.value();
    }
    for i in 0..running {
        world
            .start_fixture(
                &format!("run{i}"),
                super::fixture("dur.wait-one"),
                json!(null),
            )
            .await;
    }
    world.quiesce().await;
}

#[when(regex = r#"^an engine with the same code calls "Engine::replay_all"$"#)]
async fn same_code_replays(world: &mut LibWorld) {
    super::common::replay_all(world).await;
}

#[then("it returns an empty list")]
fn empty_list(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
}

#[given(regex = r#"^a store with running instances of a workflow that calls "A" then "B"$"#)]
async fn running_ab(world: &mut LibWorld) {
    // A and B are in each journal; the instances wait in C, so code that calls C second contradicts the history.
    world.probe.script("c", |s| s.block = true);
    for i in 0..3 {
        world
            .start_fixture(&format!("ab{i}"), super::fixture("dur.abc"), json!(null))
            .await;
    }
    let probe = Arc::clone(&world.probe);
    world
        .until("three instances reach C", || probe.calls_of("c").len() >= 3)
        .await;
}

#[when(regex = r#"^an engine whose code calls "A" then "C" calls "Engine::replay_all"$"#)]
async fn changed_code_replays(world: &mut LibWorld) {
    world.crash().await;
    world.only = Some(vec![super::variant("dur.abc.changed").register]);
    let engine = world.engine();
    world.divergences = Some(engine.replay_all().await.expect("replay_all"));
}

#[then("it returns one divergence per running instance")]
fn one_per_instance(world: &mut LibWorld) {
    assert_eq!(world.divergences.as_ref().expect("replayed").len(), 3);
}

#[then("each divergence names the step, what the journal recorded and what the code did")]
fn divergence_details(world: &mut LibWorld) {
    for divergence in world.divergences.clone().expect("replayed") {
        assert_eq!(divergence.step, 1);
        assert!(divergence.expected.contains("dur.c"), "{divergence:?}");
        let recorded = divergence.recorded.clone().unwrap_or_default();
        assert!(
            recorded.contains("dur.b") || divergence.recorded.is_none(),
            "{divergence:?}"
        );
        assert!(!divergence.call_site.is_empty());
    }
}

#[given("a running instance that started before a change")]
async fn running_before_change(world: &mut LibWorld) {
    // The instance's history reaches past the point where the new code checks the patch: B is recorded, the label
    // step is in flight.
    world.probe.script("label", |s| s.block = true);
    world
        .start_fixture("old", super::fixture("dur.carrier"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the label step runs", || probe.running("label"))
        .await;
}

#[given(regex = r#"^the new code wraps the change in "if ctx\.patched\(\\"use-new-carrier\\"\)"$"#)]
fn new_code_patch(_world: &mut LibWorld) {}

#[when("the instance resumes on the new code")]
async fn resumes_new_code(world: &mut LibWorld) {
    world.crash().await;
    world.probe.release_all();
    let patched = super::variant("dur.carrier.patched");
    world.only = Some(vec![patched.register.clone()]);
    let index = world.run_index("old");
    world.runs[index].fixture = Some(patched);
    world.resume(index).await;
}

#[then(regex = r#"^"patched\(\\"use-new-carrier\\"\)" is false for it$"#)]
async fn patched_false(world: &mut LibWorld) {
    let index = world.run_index("old");
    assert_eq!(
        world.outcome(index).await.value(),
        &json!("label via old-carrier")
    );
}

#[then("an instance started on the new code sees true")]
async fn new_sees_true(world: &mut LibWorld) {
    world
        .start_fixture("new", super::variant("dur.carrier.patched"), json!(null))
        .await;
    let index = world.run_index("new");
    assert_eq!(
        world.outcome(index).await.value(),
        &json!("label via new-carrier")
    );
}

#[then(regex = r#"^"Engine::replay_all" reports no divergence for either$"#)]
async fn no_divergence_either(world: &mut LibWorld) {
    let engine = world.engine();
    let divergences = engine.replay_all().await.expect("replay_all");
    assert!(divergences.is_empty(), "{divergences:?}");
}

#[given("a running instance whose journal does not match the current code")]
async fn mismatched(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world.until("b runs", || probe.running("b")).await;
    // Let B finish on the old code, so the journal records it.
    world.probe.release("b");
    world.probe.script("c", |s| s.block = true);
    world.until("c runs", || probe.running("c")).await;
    world.crash().await;
    world.probe.release_all();
}

#[when("it is resumed")]
async fn it_is_resumed(world: &mut LibWorld) {
    let changed = super::variant("dur.abc.changed");
    world.only = Some(vec![changed.register.clone()]);
    let last = world.last();
    world.runs[last].fixture = Some(changed);
    world.mark = Some(world.probe.count("c"));
    world.resume(last).await;
}

#[then(regex = r#"^it fails with "Error::NonDeterminism" carrying the divergence$"#)]
async fn fails_nondeterminism(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(matches!(outcome, Outcome::NonDeterminism(_)), "{outcome:?}");
}

#[then("it does not run any step past the divergence")]
fn nothing_past(world: &mut LibWorld) {
    assert_eq!(Some(world.probe.count("c")), world.mark);
}

#[given(regex = r#"^an engine that registers "shop/order@1\.0\.0" and "shop/order@2\.0\.0"$"#)]
fn both_versions(_world: &mut LibWorld) {}

#[when(regex = r#"^"Engine::recover" resumes instances of both versions$"#)]
async fn recover_both(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world
        .start_fixture("v1", super::fixture("dur.order-v1"), json!(null))
        .await;
    world
        .start_fixture("v2", super::fixture("dur.order-v2"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("both run", || probe.calls_of("b").len() >= 2)
        .await;
    world.crash().await;
    world.probe.release_all();
    world.recover().await;
}

#[then("each instance runs on the code of the version that started it")]
async fn each_own_version(world: &mut LibWorld) {
    for label in ["v1", "v2"] {
        let index = world.run_index(label);
        assert_eq!(world.outcome(index).await.value(), &json!(label));
    }
}

#[then("new executions start on the version the caller names")]
async fn caller_names(world: &mut LibWorld) {
    world
        .start_fixture("new", super::fixture("dur.order-v2"), json!(null))
        .await;
    let index = world.run_index("new");
    assert_eq!(world.outcome(index).await.value(), &json!("v2"));
}

// ---------------------------------------------------------------------------------------------------------------
// journal_persistence.feature (and the journal-growth scenario of loops.feature)
// ---------------------------------------------------------------------------------------------------------------

#[given(regex = r"^an engine with a store that records the size of every write$")]
fn recording_store(world: &mut LibWorld) {
    use_store(world, TestStore::default());
}

#[when(regex = r"^a workflow runs (\d+) task steps$")]
async fn runs_steps(world: &mut LibWorld, n: u32) {
    world
        .start_fixture("it", super::fixture("loops.count"), json!(n))
        .await;
    if !world.notes.contains_key("failing") {
        let last = world.last();
        world.outcome(last).await.value();
    }
}

#[then("each step's write is no larger than that step's own record plus a constant overhead")]
fn constant_writes(world: &mut LibWorld) {
    let store = store_of(world);
    let appends = store.sizes("append");
    let headers = store.sizes("checkpoint");
    assert!(appends.len() >= 1000, "{} appends", appends.len());
    let largest = appends.iter().max().copied().unwrap_or(0);
    assert!(largest < 2_000, "a journal append wrote {largest} bytes");
    let (small, big) = (
        headers.iter().min().copied().unwrap_or(0),
        headers.iter().max().copied().unwrap_or(0),
    );
    assert!(
        big < small * 2,
        "the journal header grew from {small} to {big} bytes"
    );
}

#[then(
    regex = r"^the total bytes written grow linearly with the (?:number of steps|iteration count)$"
)]
fn linear_total(world: &mut LibWorld) {
    let store = store_of(world);
    let writes = lock(&store.writes).clone();
    let journal: Vec<usize> = writes
        .iter()
        .filter(|(kind, _)| *kind != "event")
        .map(|(_, n)| *n)
        .collect();
    let half = journal.len() / 2;
    let first: usize = journal[..half].iter().sum();
    let second: usize = journal[half..].iter().sum();
    assert!(
        (second as f64) < (first as f64) * 1.25,
        "the second half of the run wrote {second} bytes, the first {first}"
    );
}

#[given(regex = r"^a loop of (\d+) iterations with one step each$")]
fn loop_of(world: &mut LibWorld, n: u32) {
    world.set_fixture(super::fixture("loops.count").input(json!(n)));
}

#[when("I execute it with a store that records bytes written")]
async fn execute_recording(world: &mut LibWorld) {
    use_store(world, TestStore::default());
    let input = world.fixture().input;
    world.start("it", input).await;
    let last = world.last();
    world
        .outcome_within(last, Duration::from_secs(300))
        .await
        .value();
}

#[given(regex = r"^an engine with a store that fails every write after the first (\d+)$")]
fn failing_store(world: &mut LibWorld, n: usize) {
    use_store(
        world,
        TestStore {
            fail_appends_after: Some(n),
            ..TestStore::default()
        },
    );
    world.notes.insert("failing".into(), json!(true));
}

#[then(regex = r#"^the third step fails with "Error::Persistence"$"#)]
async fn third_fails(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(
        matches!(&outcome, Outcome::Other(message) if message.contains("refuses journal writes")),
        "{outcome:?}"
    );
    assert_eq!(
        world.probe.completed("tick"),
        3,
        "the third step's task ran; recording it failed"
    );
}

#[then(regex = r#"^the instance does not report "Completed"$"#)]
async fn not_completed(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let status = world.engine().status(&id).await.expect("status");
    assert_ne!(format!("{status:?}"), "Completed");
}

#[then(regex = r#"^the failure is visible on the handle and in "Engine::status"$"#)]
async fn failure_visible(world: &mut LibWorld) {
    let last = world.last();
    let handle = world.handle(last);
    let error = handle.await.expect_err("the run failed");
    assert!(matches!(error, Error::Persistence(_)), "{error:?}");
    let id = world.runs[last].id.clone();
    let status = world.engine().status(&id).await.expect("status");
    assert_eq!(format!("{status:?}"), "Faulted");
}

#[given(regex = r"^an engine with a store that fails event writes$")]
fn failing_events(world: &mut LibWorld) {
    use_store(
        world,
        TestStore {
            fail_events: true,
            ..TestStore::default()
        },
    );
}

#[when("a workflow emits events")]
async fn emits_events(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
}

#[then(
    regex = r#"^the run fails with "Error::Persistence" or the engine retries the write until it succeeds$"#
)]
async fn run_fails_persistence(world: &mut LibWorld) {
    let last = world.last();
    let error = world.handle(last).await.expect_err("the run failed");
    assert!(matches!(error, Error::Persistence(_)), "{error:?}");
}

#[then("a warning alone is never the only outcome")]
async fn never_only_warning(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(!matches!(outcome, Outcome::Ok(_)));
}

#[given(regex = r"^an engine built with a codec that encrypts payloads with a test key$")]
fn xor_codec(world: &mut LibWorld) {
    use_store(world, TestStore::default());
    world.extras.push(Arc::new(|builder: EngineBuilder| {
        builder.codec(XorCodec(0x5a))
    }));
}

#[when(regex = r#"^a workflow runs a task with input "(.+)"$"#)]
async fn runs_with_input(world: &mut LibWorld, input: String) {
    world
        .start_fixture("it", super::fixture("dur.echo"), json!(input))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[then(regex = r#"^the raw store does not contain "(.+)"$"#)]
fn raw_store_lacks(world: &mut LibWorld, needle: String) {
    assert!(!store_of(world).contains(&needle));
}

#[then("a new engine with the same codec resumes and replays the instance")]
async fn same_codec_replays(world: &mut LibWorld) {
    super::common::replay_all(world).await;
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    let id = world.runs[world.last()].id.clone();
    let handle = (super::fixture("dur.echo").attach)(world.engine(), id)
        .await
        .expect("attach");
    assert_eq!(
        handle.await.expect("the stored result"),
        json!("secret-card-number")
    );
}

#[then("a new engine without the codec fails to read it with a clear error")]
async fn no_codec_fails(world: &mut LibWorld) {
    world.crash().await;
    world.extras.clear();
    let id = world.runs[world.last()].id.clone();
    let error = (super::fixture("dur.echo").attach)(world.engine(), id)
        .await
        .expect_err("unreadable");
    assert!(error.to_string().contains("test-xor"), "{error}");
}

#[given(regex = r"^an engine built with a compressing codec$")]
fn rle_codec(world: &mut LibWorld) {
    use_store(world, TestStore::default());
    world
        .extras
        .push(Arc::new(|builder: EngineBuilder| builder.codec(RleCodec)));
}

#[when(regex = r"^a task returns a 5 MiB JSON value$")]
async fn five_mib(world: &mut LibWorld) {
    let big = "x".repeat(5 * 1024 * 1024);
    world.notes.insert("big".into(), json!(big.len()));
    world
        .start_fixture("it", super::fixture("dur.echo"), json!(big))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[then("the stored record is smaller than 5 MiB")]
fn stored_small(world: &mut LibWorld) {
    let store = store_of(world);
    let largest = lock(&store.writes)
        .iter()
        .map(|(_, n)| *n)
        .max()
        .unwrap_or(0);
    assert!(largest < 512 * 1024, "a write of {largest} bytes");
}

#[then("the step's output after a resume equals the original value")]
async fn output_after_resume(world: &mut LibWorld) {
    world.crash().await;
    let id = world.runs[world.last()].id.clone();
    let handle = (super::fixture("dur.echo").attach)(world.engine(), id)
        .await
        .expect("attach");
    let value = handle.await.expect("the stored result");
    assert_eq!(value.as_str().map(str::len), Some(5 * 1024 * 1024));
}

#[given("a journal written by the previous minor release of jackdaw")]
async fn old_journal(world: &mut LibWorld) {
    let store = use_store(world, TestStore::default());
    let input = json!(1);
    let hash: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(serde_json::to_vec(&input).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let journal = json!({
        "workflow": { "namespace": "dur", "name": "legacy", "version": "1" },
        "input": null,
        "status": "Running",
        "steps": { "0": {
            "kind": "call", "name": "dur.second", "ordinal": 0, "input_hash": hash,
            "call_site": "an older build", "outcome": { "Ok": 1 }
        }},
        "signals": [],
        "patches": {},
        "result": null
    });
    store
        .save_checkpoint(WorkflowCheckpoint {
            instance_id: "legacy-1".into(),
            current_task: "dur.second".into(),
            data: json!({ "jackdaw_native": journal }),
            timestamp: chrono::Utc::now(),
        })
        .await
        .expect("seed the old journal");
    world.notes.insert("legacy".into(), json!("legacy-1"));
}

#[when("the current release resumes it")]
async fn current_resumes(world: &mut LibWorld) {
    let handle = (super::fixture("dur.legacy").resume)(world.engine(), "legacy-1".into())
        .await
        .expect("resume");
    let now = world.clock.now();
    world.push_run("it", super::fixture("dur.legacy"), handle, now);
}

#[then("the instance completes")]
async fn instance_completes(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(3));
    let inputs: Vec<Value> = world
        .probe
        .calls_of("dur_second")
        .into_iter()
        .map(|c| c.input)
        .collect();
    assert_eq!(inputs, vec![json!(2)], "the recorded step is not run again");
}

#[then("the store records which journal format version each instance uses")]
async fn format_recorded(world: &mut LibWorld) {
    let checkpoint = store_of(world)
        .get_checkpoint("legacy-1")
        .await
        .expect("read")
        .expect("a checkpoint");
    assert_eq!(checkpoint.data["journal_format"], json!(2));
    let _ = Instant::now();
    let _ = Duration::ZERO;
    let _: Option<ExecuteOptions> = None;
}
