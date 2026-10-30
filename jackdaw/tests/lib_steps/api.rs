//! api/*.feature: tasks, workflows, determinism, the builder, events, signals and updates, the public surface.
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::gherkin::Step as GherkinStep;
use cucumber::{given, then, when};
use jackdaw::api::{ListFilter, Store};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------------------------------------------
// Tasks and workflows
// ---------------------------------------------------------------------------------------------------------------

pub struct Api(pub Arc<Probe>);

#[tasks]
impl Api {
    #[task(name = "api.double")]
    pub async fn double(self: Arc<Self>, ctx: TaskContext, n: u64) -> Result<u64, TaskError> {
        self.0.typed("api_double", &ctx, &n).await
    }
    #[task(name = "payments.charge")]
    pub async fn charge(
        self: Arc<Self>,
        ctx: TaskContext,
        cents: Value,
    ) -> Result<String, TaskError> {
        self.0.typed("api_payments", &ctx, &cents).await
    }
    #[task(name = "api.inc_a")]
    pub async fn inc_a(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("api_inc", &ctx, &()).await
    }
    #[task(name = "api.inc_b")]
    pub async fn inc_b(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("api_inc", &ctx, &()).await
    }
    #[task(name = "api.cached", cached)]
    pub async fn cached(self: Arc<Self>, ctx: TaskContext, n: u64) -> Result<u64, TaskError> {
        self.0.typed("api_cached", &ctx, &n).await
    }
    #[task(name = "api.plain")]
    pub async fn plain(self: Arc<Self>, ctx: TaskContext, n: u64) -> Result<u64, TaskError> {
        self.0.typed("api_plain", &ctx, &n).await
    }
    /// Heartbeats twice, then goes quiet on its first attempt; later attempts return the last progress they see.
    #[task(name = "api.beating")]
    pub async fn beating(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<u32, TaskError> {
        self.0.invoke("api_beating", &ctx, json!(null)).await?;
        if ctx.attempt() > 1 {
            return Ok(ctx.last_heartbeat::<u32>().unwrap_or(0));
        }
        let clock = self.0.clock.clone();
        for progress in 1..=2u32 {
            ctx.heartbeat(progress);
            jackdaw::api::Clock::sleep_until(&clock, clock.now() + chrono::Duration::seconds(1))
                .await;
        }
        std::future::pending().await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("api_double", |n| json!(n.as_u64().unwrap_or(0) * 2));
    probe.default_output("api_payments", |cents| json!(format!("paid {cents}")));
    probe.default_output("api_inc", |_| json!(null));
    probe.default_output("api_cached", |n| n.clone());
    probe.default_output("api_plain", |n| n.clone());
    probe.default_output("api_beating", |_| json!(null));
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QsOrder {
    pub id: String,
    pub cents: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QsShipment {
    pub tracking: String,
}

#[task(name = "qs.charge")]
pub async fn charge(order: QsOrder) -> Result<String, TaskError> {
    Ok(format!("ch-{}", order.id))
}

#[task(name = "qs.ship")]
pub async fn ship(charge_id: String) -> Result<QsShipment, TaskError> {
    Ok(QsShipment {
        tracking: format!("trk-{charge_id}"),
    })
}

/// The quickstart from the crate documentation.
#[workflow(namespace = "shop", name = "order", version = "quickstart")]
pub struct OrderWorkflow;

#[workflow_methods]
impl OrderWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: QsOrder) -> WorkflowResult<QsShipment> {
        let charge_id = ctx
            .call(Charge, &order)
            .retry(Retry::exponential(5))
            .timeout(30.secs())
            .await?;
        Ok(ctx.call(Ship, &charge_id).await?)
    }
}

#[workflow(namespace = "api", name = "double", version = "1")]
pub struct DoubleOnce;

#[workflow_methods]
impl DoubleOnce {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, n: u64) -> WorkflowResult<u64> {
        Ok(ctx.call(Api::double, &n).await?)
    }
}

#[workflow(namespace = "api", name = "both", version = "1")]
pub struct Both;

#[workflow_methods]
impl Both {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.call(Api::inc_a, &()).await?;
        Ok(ctx.call(Api::inc_b, &()).await?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum CacheUse {
    Cached,
    Plain,
    PlainButCached,
    CachedButUncached,
}

#[workflow(namespace = "api", name = "cache", version = "1")]
pub struct CacheUser;

#[workflow_methods]
impl CacheUser {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, how: CacheUse) -> WorkflowResult<u64> {
        Ok(match how {
            CacheUse::Cached => ctx.call(Api::cached, &7).await?,
            CacheUse::Plain => ctx.call(Api::plain, &7).await?,
            CacheUse::PlainButCached => ctx.call(Api::plain, &7).cached().await?,
            CacheUse::CachedButUncached => ctx.call(Api::cached, &7).uncached().await?,
        })
    }
}

#[workflow(namespace = "api", name = "retry-constant", version = "1")]
pub struct RetryConstant;

#[workflow_methods]
impl RetryConstant {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
        Ok(ctx
            .call(super::errors::Errs::charge, &cents)
            .retry(Retry::constant(1.millis()).max_attempts(3))
            .await?)
    }
}

#[workflow(namespace = "api", name = "handled", version = "1")]
pub struct Handled;

#[workflow_methods]
impl Handled {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
        Ok(
            match ctx
                .call(super::errors::Errs::charge, &cents)
                .retry(Retry::exponential(5))
                .await
            {
                Ok(id) => id,
                Err(StepError::Fault(fault)) => {
                    format!("handled: {}", fault.detail.unwrap_or_default())
                }
                Err(other) => return Err(other),
            },
        )
    }
}

#[workflow(namespace = "api", name = "timeout", version = "1")]
pub struct TimeoutPropagates;

#[workflow_methods]
impl TimeoutPropagates {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(
            match ctx
                .call(super::errors::Errs::slow, &())
                .timeout(1.secs())
                .await
            {
                Ok(()) => "ok".into(),
                Err(StepError::Timeout { .. }) => "StepError::Timeout".into(),
                Err(other) => return Err(other),
            },
        )
    }
}

#[workflow(namespace = "api", name = "heartbeats", version = "1")]
pub struct Heartbeats;

#[workflow_methods]
impl Heartbeats {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        Ok(ctx
            .call(Api::beating, &())
            .heartbeat_timeout(3.secs())
            .retry(Retry::constant(1.millis()).max_attempts(2))
            .await?)
    }
}

#[workflow(namespace = "api", name = "race", version = "1")]
pub struct RaceApproved;

#[workflow_methods]
impl RaceApproved {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(jackdaw::select! {
            _ = ctx.wait_for_event::<Value>("approved") => "event".to_string(),
            _ = ctx.sleep(1.secs()) => "timer".to_string(),
        })
    }
}

#[workflow(namespace = "api", name = "read", version = "1")]
pub struct ReadAndReturn;

#[workflow_methods]
impl ReadAndReturn {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, which: String) -> WorkflowResult<Value> {
        Ok(match which.as_str() {
            "ctx.now()" => json!(ctx.now()),
            "ctx.random()" => json!(ctx.random()),
            _ => json!(ctx.uuid()),
        })
    }
}

#[workflow(namespace = "api", name = "counter", version = "1")]
pub struct StepCounter {
    done: u32,
}

#[workflow_methods]
impl StepCounter {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { done: 0 }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        for n in 1..=3u64 {
            ctx.call(Api::double, &n).await?;
            ctx.state_mut(|s| s.done += 1);
        }
        Ok(ctx.state(|s| s.done))
    }

    #[query]
    fn done(&self, _ctx: &WorkflowContextView) -> u32 {
        self.done
    }
}

#[workflow(namespace = "api", name = "batches", version = "1")]
pub struct Batches;

#[workflow_methods]
impl Batches {
    #[run]
    async fn run(
        ctx: &mut WorkflowContext<Self>,
        (cursor, total): (u64, u64),
    ) -> WorkflowResult<u64> {
        ctx.call(Api::double, &cursor).await?;
        if cursor + 1 == total {
            return Ok(total);
        }
        ctx.continue_as_new((cursor + 1, total))
    }
}

#[workflow(namespace = "api", name = "approval", version = "1")]
pub struct Approval {
    approved: bool,
}

#[workflow_methods]
impl Approval {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { approved: false }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        ctx.wait_condition(|s| s.approved).await?;
        Ok("approved".into())
    }

    #[signal]
    pub fn approve(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.approved = true;
    }

    #[query]
    pub fn is_approved(&self, _ctx: &WorkflowContextView) -> bool {
        self.approved
    }
}

#[workflow(namespace = "api", name = "limits", version = "1")]
pub struct Limits {
    limit: u64,
    applied: u32,
    done: bool,
}

#[workflow_methods]
impl Limits {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self {
            limit: 10,
            applied: 0,
            done: false,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        ctx.wait_condition(|s| s.done).await?;
        Ok(ctx.state(|s| s.limit))
    }

    #[update]
    pub fn set_limit(
        &mut self,
        _ctx: &mut SyncWorkflowContext<Self>,
        limit: u64,
    ) -> Result<u64, UpdateError> {
        if limit > 100 {
            return Err(UpdateError::new(format!("limit {limit} is above 100")));
        }
        self.applied += 1;
        Ok(std::mem::replace(&mut self.limit, limit))
    }

    #[signal]
    pub fn finish(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.done = true;
    }

    #[query]
    pub fn limit(&self, _ctx: &WorkflowContextView) -> (u64, u32) {
        (self.limit, self.applied)
    }
}

#[workflow(namespace = "api", name = "sleep-then-wait", version = "1")]
pub struct SecondThenWait;

#[workflow_methods]
impl SecondThenWait {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        ctx.sleep(1.secs()).await?;
        let paid: Value = ctx.wait_for_event("paid").await?;
        Ok(paid["cents"].as_u64().unwrap_or(0))
    }
}

#[workflow(namespace = "api", name = "four-tasks", version = "1")]
pub struct FourTasks;

#[workflow_methods]
impl FourTasks {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        let mut total = 0;
        for n in 1..=4u64 {
            total += ctx.call(Api::double, &n).await?;
        }
        Ok(total)
    }
}

pub mod reordered {
    use super::*;
    use crate::lib_steps::durability::Durable;

    #[workflow(namespace = "dur", name = "abc", version = "1")]
    pub struct Abc;

    #[workflow_methods]
    impl Abc {
        #[run]
        async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<String>> {
            let b = ctx.call(Durable::b, &()).await?;
            let a = ctx.call(Durable::a, &()).await?;
            let c = ctx.call(Durable::c, &()).await?;
            Ok(vec![b, a, c])
        }
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<OrderWorkflow>("api.quickstart").input(json!({"id": "o-1", "cents": 500})),
        Fixture::of::<DoubleOnce>("api.double").input(json!(21)),
        Fixture::of::<Both>("api.both"),
        Fixture::of::<CacheUser>("api.cache").input(json!("Cached")),
        Fixture::of::<RetryConstant>("api.retry-constant").input(json!(500)),
        Fixture::of::<Handled>("api.handled").input(json!(500)),
        Fixture::of::<TimeoutPropagates>("api.timeout"),
        Fixture::of::<Heartbeats>("api.heartbeats"),
        Fixture::of::<RaceApproved>("api.race"),
        Fixture::of::<ReadAndReturn>("api.read"),
        Fixture::of::<StepCounter>("api.counter"),
        Fixture::of::<Batches>("api.batches"),
        Fixture::of::<Approval>("api.approval"),
        Fixture::of::<Limits>("api.limits"),
        Fixture::of::<SecondThenWait>("api.second-then-wait"),
        Fixture::of::<FourTasks>("api.four-tasks"),
    ]
}

pub fn variants() -> Vec<Fixture> {
    vec![Fixture::of::<reordered::Abc>("api.abc.reordered")]
}

pub fn task_sets(
    probe: &Arc<Probe>,
) -> Vec<Box<dyn Fn(EngineBuilder) -> EngineBuilder + Send + Sync>> {
    let p = Arc::clone(probe);
    vec![
        Box::new(move |b| b.tasks(Api(Arc::clone(&p)))),
        Box::new(|b| b.task(Charge).task(Ship)),
    ]
}

fn ui(world: &mut LibWorld, case: &str) {
    world.notes.insert("ui".into(), json!(case));
}

fn stderr(world: &LibWorld) -> String {
    let case = world.notes["ui"].as_str().expect("a case").to_string();
    let path = format!("{}/tests/rust/ui/{case}.stderr", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

// ---------------------------------------------------------------------------------------------------------------
// api/tasks.feature
// ---------------------------------------------------------------------------------------------------------------

#[given("the task:")]
fn the_task(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.double"));
}

#[when(regex = r#"^a workflow calls "Double" with (\d+)$"#)]
async fn calls_double(world: &mut LibWorld, n: u64) {
    world
        .start_fixture("it", super::fixture("api.double"), json!(n))
        .await;
}

#[then(regex = r"^the step returns (\d+)$")]
async fn step_returns_number(world: &mut LibWorld, n: u64) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(n));
}

#[given(
    regex = r#"^the task "charge" declared with "\#\[task\(name = \\"payments\.charge\\"\)\]"$"#
)]
fn named_task(_world: &mut LibWorld) {}

#[then(regex = r#"^it is registered as "([\w.]+)"$"#)]
fn registered_as(world: &mut LibWorld, name: String) {
    // A YAML workflow that calls an unknown task is rejected; one that calls this name is accepted.
    world.notes.insert("task-name".into(), json!(name));
}

#[then(regex = r#"^a YAML workflow can "call: ([\w.]+)"$"#)]
async fn yaml_calls(world: &mut LibWorld, name: String) {
    let yaml = format!(
        "document:\n  dsl: '1.0.3'\n  namespace: api\n  name: calls-native\n  version: '1.0.0'\ndo:\n  - pay:\n      call: {name}\n      with:\n        cents: 5\n"
    );
    let workflow = Workflow::<Value, Value>::from_yaml(&yaml).expect("YAML");
    let engine = world.engine();
    let output = engine
        .execute(&workflow, json!({}))
        .await
        .expect("starts")
        .await
        .expect("runs");
    assert_eq!(world.probe.count("api_payments"), 1, "{output}");
}

#[given(regex = r#"^a type "Shop" with a shared counter and "\#\[tasks\]" on its impl$"#)]
fn shop_type(_world: &mut LibWorld) {}

#[given(regex = r#"^two "\#\[task\]" methods that each increment the counter$"#)]
fn two_methods(_world: &mut LibWorld) {}

#[when("a workflow calls both methods")]
async fn calls_both(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.both"), json!(null))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[then(regex = r"^the counter is (\d+)$")]
fn counter_is(world: &mut LibWorld, n: usize) {
    assert_eq!(world.probe.count("api_inc"), n);
}

#[given(
    regex = r#"^a synchronous function marked "\#\[task\(blocking\)\]" that sleeps the thread for 200 ms$"#
)]
fn blocking_200(_world: &mut LibWorld) {}

#[when("a workflow calls it while another workflow is waiting on a 10 ms timer")]
async fn blocking_and_timer(world: &mut LibWorld) {
    world
        .start_fixture("grinder", super::fixture("calls.grinder"), json!(null))
        .await;
    world
        .start_fixture("napper", super::fixture("calls.napper"), json!(null))
        .await;
}

#[then("the other workflow's timer fires before the blocking task finishes")]
async fn timer_first(world: &mut LibWorld) {
    let napper = world.run_index("napper");
    world.outcome(napper).await.value();
    let grinder = world.run_index("grinder");
    world.outcome(grinder).await.value();
    assert!(world.runs[napper].finished_at < world.runs[grinder].finished_at);
}

#[given(regex = r#"^"\#\[task\(blocking\)\]" on an "async fn"$"#)]
fn blocking_async(world: &mut LibWorld) {
    ui(world, "misuse_blocking_async");
}

#[then(regex = r#"^it fails with "([^"]+)"$"#)]
fn it_fails_with(world: &mut LibWorld, message: String) {
    if let Some(variant) = message.strip_prefix("Error::") {
        // A runtime error the scenario recorded, by its variant.
        let error = world.notes["error"].as_str().expect("an error").to_string();
        assert!(error.starts_with(&format!("{variant}(")), "{error}");
        return;
    }
    let stderr = stderr(world);
    assert!(stderr.contains(&message), "{stderr}");
}

#[given(regex = r#"^a workflow that passes a "String" to a task that takes "u64"$"#)]
fn passes_string(world: &mut LibWorld) {
    ui(world, "mismatched_task_input");
}

#[then("it fails to compile with a type mismatch on the call")]
fn type_mismatch_call(world: &mut LibWorld) {
    let stderr = stderr(world);
    let mismatch = stderr.contains("error[E0308]") || stderr.contains("error[E0277]");
    assert!(mismatch && stderr.contains("ctx.call(Greet"), "{stderr}");
}

#[given(regex = r#"^a task that fails with "TaskError::retryable" on its first (\d+) attempts$"#)]
fn retryable_first(world: &mut LibWorld, n: u32) {
    world.probe.script("charge", |s| s.fail_first = n);
}

#[when(regex = r#"^a workflow calls it with "Retry::exponential\(5\)"$"#)]
async fn calls_exponential(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("err.retrying"), json!(500))
        .await;
}

#[then(regex = r#"^the event stream contains (\d+) "TaskRetried" events$"#)]
async fn stream_retried(world: &mut LibWorld, n: usize) {
    let last = world.last();
    world.outcome(last).await;
    let mut events = world.handle(last).events();
    let mut retried = 0;
    while let Some(event) = events.next().await {
        retried += usize::from(matches!(event, Event::TaskRetried { .. }));
    }
    assert_eq!(retried, n);
}

#[then(regex = r#"^"TaskContext::attempt" was 1, 2 and 3 on the three attempts$"#)]
fn attempts_123(world: &mut LibWorld) {
    let attempts: Vec<u32> = world
        .probe
        .calls_of("charge")
        .iter()
        .map(|c| c.attempt)
        .collect();
    assert_eq!(attempts, vec![1, 2, 3]);
}

#[given(regex = r#"^a task that fails with "TaskError::non_retryable"$"#)]
fn non_retryable_task(world: &mut LibWorld) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::non_retryable("declined"))
    });
}

#[then(regex = r#"^the workflow can handle the failure with "\?" or "match"$"#)]
async fn handled_failure(world: &mut LibWorld) {
    world
        .start_fixture("handled", super::fixture("api.handled"), json!(500))
        .await;
    let index = world.run_index("handled");
    assert_eq!(
        world.outcome(index).await.value(),
        &json!("handled: declined")
    );
}

#[given(regex = r#"^a task that always fails with "TaskError::retryable"$"#)]
fn always_retryable(world: &mut LibWorld) {
    world.probe.script("charge", |s| s.fail_first = u32::MAX);
}

#[when(
    regex = r#"^a workflow calls it with "Retry::constant\(1\.millis\(\)\)\.max_attempts\(3\)"$"#
)]
async fn calls_constant(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.retry-constant"), json!(500))
        .await;
}

#[given(
    regex = r#"^a task that fails with "TaskError::retryable\(\.\.\)\.next_retry_delay\(5\.secs\(\)\)"$"#
)]
fn asks_for_delay(world: &mut LibWorld) {
    world.probe.script("charge", |s| {
        s.fail_on.push((
            json!(500),
            FaultSpec {
                next_retry_delay: Some(Duration::from_secs(5)),
                ..FaultSpec::retryable("busy")
            },
            Some(1),
        ));
    });
}

#[when(regex = r"^a workflow calls it with a retry policy(?: on a virtual clock)?$")]
async fn calls_with_policy(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("err.retrying"), json!(500))
        .await;
}

#[then(regex = r"^the next attempt starts (\d+) seconds later$")]
async fn next_attempt_later(world: &mut LibWorld, seconds: i64) {
    let last = world.last();
    world.outcome(last).await.value();
    let calls = world.probe.calls_of("charge");
    assert_eq!(
        (calls[1].started_at - calls[0].started_at).num_seconds(),
        seconds
    );
}

#[when(regex = r#"^a workflow calls it with "\.timeout\(1\.secs\(\)\)"$"#)]
async fn calls_with_timeout(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.timeout"), json!(null))
        .await;
}

#[then(regex = r#"^the step fails with "StepError::Timeout"$"#)]
async fn fails_with_timeout(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!("StepError::Timeout")
    );
}

#[then(regex = r#"^the step fails with a fault whose detail contains "(.+)"$"#)]
async fn fault_detail(world: &mut LibWorld, text: String) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert!(fault.detail.unwrap_or_default().contains(&text));
}

#[then("the engine keeps running other workflows")]
async fn keeps_running(world: &mut LibWorld) {
    world
        .start_fixture("other", super::fixture("api.double"), json!(2))
        .await;
    let other = world.run_index("other");
    assert_eq!(world.outcome(other).await.value(), &json!(4));
}

#[given(
    regex = r#"^a task that records "TaskContext::idempotency_key" on every attempt and fails the first one$"#
)]
fn key_every_attempt(world: &mut LibWorld) {
    world.probe.script("charge", |s| s.fail_first = 1);
}

#[when("the instance is replayed from its journal")]
async fn replayed_from_journal(world: &mut LibWorld) {
    let last = world.last();
    if world.runs[last]
        .fixture
        .as_ref()
        .is_some_and(|f| f.name == "api.limits")
    {
        // A running instance: rebuilt from its journal by a new engine.
        world.crash().await;
        world.resume(last).await;
    } else {
        world.outcome(last).await;
        super::common::replay_all(world).await;
    }
}

#[then("every recorded key is the same")]
fn same_keys(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    let keys: std::collections::HashSet<String> = world
        .probe
        .calls_of("charge")
        .into_iter()
        .map(|c| c.key)
        .collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(world.probe.count("charge"), 2);
}

#[given(
    regex = r#"^a task declared with "\#\[task\(cached\)\]"(?: that counts its invocations)?$"#
)]
fn cached_task(world: &mut LibWorld) {
    world
        .notes
        .insert("cache-use".into(), json!(CacheUse::Cached));
}

#[given(regex = r#"^a task declared with "\#\[task\]" that counts its invocations$"#)]
fn plain_task(world: &mut LibWorld) {
    world
        .notes
        .insert("cache-use".into(), json!(CacheUse::Plain));
}

#[when(
    regex = r#"^two separate workflow instances call it with the same input( using "\.cached\(\)")?$"#
)]
async fn two_instances(world: &mut LibWorld, opt_in: String) {
    let mut how: CacheUse =
        serde_json::from_value(world.notes["cache-use"].clone()).expect("a task");
    if !opt_in.is_empty() {
        how = CacheUse::PlainButCached;
    }
    for label in ["first", "second"] {
        world
            .start_fixture(label, super::fixture("api.cache"), json!(how))
            .await;
        let index = world.run_index(label);
        world.outcome(index).await.value();
    }
}

#[then(regex = r"^the task ran (once|twice)$")]
fn task_ran(world: &mut LibWorld, times: String) {
    let expected = if times == "once" { 1 } else { 2 };
    let total = world.probe.count("api_cached") + world.probe.count("api_plain");
    assert_eq!(total, expected);
}

#[when(regex = r#"^a workflow calls it with "\.uncached\(\)"$"#)]
async fn calls_uncached(world: &mut LibWorld) {
    for (label, how) in [
        ("cached", CacheUse::Cached),
        ("uncached", CacheUse::CachedButUncached),
    ] {
        world
            .start_fixture(label, super::fixture("api.cache"), json!(how))
            .await;
        let index = world.run_index(label);
        world.outcome(index).await.value();
    }
}

#[then("the task runs even though an equal input was cached")]
fn runs_despite_cache(world: &mut LibWorld) {
    assert_eq!(world.probe.count("api_cached"), 2);
}

#[when(
    regex = r#"^I call "Engine::run_task\(Double, 21, TaskOptions::new\(\)\.retry\(Retry::exponential\(3\)\)\)"$"#
)]
async fn run_task(world: &mut LibWorld) {
    let engine = world.engine();
    let probe = Arc::clone(&world.probe);
    let _ = probe;
    let out = engine
        .run_task(
            Api::double,
            21,
            TaskOptions::new().retry(Retry::exponential(3)),
        )
        .await
        .expect("runs");
    world.notes.insert("run_task".into(), json!(out));
}

#[then(regex = r"^it returns (\d+)$")]
fn it_returns(world: &mut LibWorld, n: u64) {
    assert_eq!(world.notes["run_task"], json!(n));
}

#[then("no workflow instance is recorded in the store")]
async fn no_instance(world: &mut LibWorld) {
    let listed = world
        .engine()
        .list(&ListFilter::new().include_children(true))
        .await
        .expect("list");
    assert!(listed.is_empty(), "{listed:?}");
}

#[given(regex = r#"^a task that calls "TaskContext::heartbeat\(progress\)" every second$"#)]
fn heartbeating(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.heartbeats"));
}

#[given(regex = r#"^the step has "\.heartbeat_timeout\(3\.secs\(\)\)"$"#)]
fn heartbeat_timeout(_world: &mut LibWorld) {}

#[when(regex = r"^the task stops heartbeating for 3 seconds$")]
async fn stops_heartbeating(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
}

#[then("the attempt fails with a heartbeat timeout and is retried")]
async fn heartbeat_retried(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    let attempts: Vec<u32> = world
        .probe
        .calls_of("api_beating")
        .iter()
        .map(|c| c.attempt)
        .collect();
    assert_eq!(attempts, vec![1, 2]);
}

#[then(
    regex = r#"^the next attempt can read the last progress with "TaskContext::last_heartbeat"$"#
)]
async fn reads_progress(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(2));
}

#[when("the instance is cancelled while the task runs")]
async fn cancelled_while_running(world: &mut LibWorld) {
    world.handle(world.last()).cancel().await.expect("cancel");
}

#[then("the task's cancellation future resolves")]
async fn cancellation_resolves(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the task stopped", || {
            probe.calls_of("poll").iter().all(|c| c.ended.is_some())
        })
        .await;
}

#[then(regex = r#"^the instance ends with status "(\w+)"$"#)]
async fn ends_with_status(world: &mut LibWorld, status: String) {
    let id = world.runs[world.last()].id.clone();
    let actual = world.engine().status(&id).await.expect("status");
    assert_eq!(format!("{actual:?}"), status);
}

// ---------------------------------------------------------------------------------------------------------------
// api/workflows.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(
    regex = r#"^the "OrderWorkflow" from the crate documentation with tasks "Charge" and "Ship"$"#
)]
fn quickstart(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.quickstart"));
}

#[when(regex = r#"^I execute "OrderWorkflow::run" with order "([\w-]+)"$"#)]
async fn execute_quickstart(world: &mut LibWorld, id: String) {
    world.start("it", json!({ "id": id, "cents": 500 })).await;
}

#[when("I await the handle")]
async fn await_handle(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await;
}

#[then(regex = r#"^the result is a shipment with tracking "(.+)"$"#)]
async fn shipment_tracking(world: &mut LibWorld, tracking: String) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({ "tracking": tracking })
    );
}

#[given("a workflow that waits for a signal before finishing")]
fn waits_for_signal(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.approval"));
}

#[then("I get a handle immediately")]
fn handle_immediately(world: &mut LibWorld) {
    let last = world.last();
    assert!(world.runs[last].handle.is_some() && world.runs[last].outcome.is_none());
}

#[then(regex = r#"^"ExecutionHandle::id" is a non-empty instance id$"#)]
fn non_empty_id(world: &mut LibWorld) {
    assert!(!world.runs[world.last()].id.is_empty());
}

#[then(regex = r#"^"Engine::status" of (?:that id|the instance) is "(\w+)"$"#)]
async fn status_of(world: &mut LibWorld, status: String) {
    let id = world.runs[world.last()].id.clone();
    let actual = world.engine().status(&id).await.expect("status");
    assert_eq!(format!("{actual:?}"), status);
}

#[given(regex = r#"^a workflow whose "\#\[run\]" returns "WorkflowResult<u64>"$"#)]
fn returns_u64(world: &mut LibWorld) {
    ui(world, "mismatched_workflow_output");
}

#[when(regex = r#"^the caller awaits the handle as "String"$"#)]
fn awaits_as_string(_world: &mut LibWorld) {}

#[then("it fails to compile with a type mismatch")]
fn compile_mismatch(world: &mut LibWorld) {
    assert!(stderr(world).contains("E0308"));
}

#[given(regex = r#"^a "\#\[workflow_methods\]" impl without "\#\[run\]"$"#)]
fn without_run(world: &mut LibWorld) {
    ui(world, "misuse_missing_run");
}

#[given(regex = r#"^a parent workflow that runs the child "Refund" with input (\d+)$"#)]
fn parent_with_child(world: &mut LibWorld, n: u64) {
    world.set_fixture(super::fixture("kids.order").input(json!(n)));
}

#[when("I execute the parent")]
async fn execute_parent(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
}

#[then("the child runs as its own instance")]
async fn child_own_instance(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    let parent = world.runs[last].id.clone();
    let child = world.probe.calls_of("pay")[0].instance.clone();
    assert!(child != parent && child.starts_with(&parent));
}

#[then("the parent's result contains the child's output")]
async fn parent_contains(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({ "charge": "ch-5" })
    );
}

#[given("a child workflow that fails with a validation fault")]
fn child_fails(world: &mut LibWorld) {
    world.probe.script("pay", |s| {
        s.fault = Some(FaultSpec::kind("validation").title("bad amount"))
    });
}

#[when("the parent runs it")]
async fn parent_runs(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("kids.order"), json!(5))
        .await;
}

#[then("the parent's step fails with that fault")]
async fn parent_fault(world: &mut LibWorld) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.title.as_deref(), Some("bad amount"));
}

#[given("a task that tracks how many copies of it run at the same time")]
fn tracks_copies(_world: &mut LibWorld) {}

#[then(regex = r"^all (\d+) results come back in input order$")]
async fn all_results(world: &mut LibWorld, n: u32) {
    let last = world.last();
    let expected: Vec<u32> = (0..n).collect();
    assert_eq!(world.outcome(last).await.value(), &json!(expected));
}

#[then(regex = r"^no more than (\d+) copies ran at the same time$")]
fn no_more_than(world: &mut LibWorld, n: usize) {
    assert!(world.probe.max_in_flight("task") <= n);
}

#[given(
    regex = r#"^a workflow that races a 1-second sleep against "wait_for_event\(\\"approved\\"\)"$"#
)]
fn races(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.race"));
}

#[when(regex = r#"^the event "approved" arrives after (\d+) ms$"#)]
async fn approved_after(world: &mut LibWorld, ms: u64) {
    world.start("it", json!(null)).await;
    world.pass(Duration::from_millis(ms)).await;
    world
        .handle(world.last())
        .send_event("approved", json!({}))
        .await
        .expect("send");
}

#[then("the race resolves to the event side")]
async fn event_side(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("event"));
}

#[then("replaying the instance takes the same side")]
async fn same_side(world: &mut LibWorld) {
    let divergences = world.engine().replay_all().await.expect("replay_all");
    assert!(divergences.is_empty());
}

#[given(regex = r#"^a workflow that reads "(ctx\.\w+\(\))" and returns it$"#)]
fn reads_and_returns(world: &mut LibWorld, call: String) {
    world.set_fixture(super::fixture("api.read").input(json!(call)));
}

#[when("I execute it and record the result")]
async fn execute_record(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    world.notes.insert("recorded".into(), value);
}

#[when("I replay the instance from its journal")]
async fn replay_from_journal(world: &mut LibWorld) {
    let engine = world.engine();
    world.divergences = Some(engine.replay_all().await.expect("replay_all"));
    let id = world.runs[world.last()].id.clone();
    let handle = (super::fixture("api.read").attach)(engine, id)
        .await
        .expect("attach");
    world
        .notes
        .insert("replayed".into(), handle.await.expect("the stored result"));
}

#[then("the replay returns the same value")]
fn replay_same(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    assert_eq!(world.notes["replayed"], world.notes["recorded"]);
}

#[given(regex = r#"^a workflow with a "\#\[query\]" that returns its step counter$"#)]
fn step_counter(world: &mut LibWorld) {
    world
        .probe
        .script("api_double", |s| s.block_on = Some(json!(3)));
    world.set_fixture(super::fixture("api.counter"));
}

#[when(regex = r"^the workflow has finished 2 of 3 steps$")]
async fn finished_two(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the third step runs", || probe.running("api_double"))
        .await;
}

#[then(regex = r"^querying it returns (\d+)$")]
async fn querying_returns(world: &mut LibWorld, n: u32) {
    let handle = world.handle(world.last());
    assert_eq!(handle.query(StepCounter::done).await.expect("query"), n);
    world.probe.release_all();
}

#[given(
    regex = r#"^a workflow that processes one batch and then calls "ctx\.continue_as_new\(next_cursor\)"$"#
)]
fn batches(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.batches"));
}

#[when(regex = r"^it has processed (\d+) batches$")]
async fn processed(world: &mut LibWorld, n: u64) {
    world.start("it", json!([0, n])).await;
    let last = world.last();
    world
        .outcome_within(last, Duration::from_secs(120))
        .await
        .value();
}

#[then("the current run's journal holds the steps of one batch only")]
async fn one_batch(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert!(details.steps <= 1, "{details:?}");
}

#[then("all runs share the same workflow instance id")]
fn same_id(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    assert!(
        world
            .probe
            .calls_of("api_double")
            .iter()
            .all(|c| c.instance == id)
    );
}

#[then(regex = r#"^"Engine::history" can follow the chain of runs$"#)]
async fn chain(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    let runs: Vec<u32> = history
        .iter()
        .filter_map(|e| match e {
            Event::ContinuedAsNew { run, .. } => Some(*run),
            _ => None,
        })
        .collect();
    assert_eq!(runs, (2..=1000).collect::<Vec<u32>>());
}

#[then("the handle the caller holds resolves when the last run completes")]
async fn handle_resolves(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(1000));
}

// ---------------------------------------------------------------------------------------------------------------
// api/determinism.feature and api/public_surface.feature
// ---------------------------------------------------------------------------------------------------------------

#[then(regex = r#"^it fails with a message that names "([^"]+)" and suggests "([^"]+)"$"#)]
fn names_and_suggests(world: &mut LibWorld, call: String, advice: String) {
    let stderr = stderr(world);
    assert!(
        stderr.contains(&format!("`{call}")) && stderr.contains(&advice),
        "{stderr}"
    );
}

#[then(regex = r#"^the message says audited code can opt out with "(.+)"$"#)]
fn opt_out(world: &mut LibWorld, attribute: String) {
    assert!(stderr(world).contains(&attribute));
}

#[given(
    regex = r#"^a "\#\[run\]" body that calls "SystemTime::now" in a statement marked "\#\[allow_nondeterminism\]"$"#
)]
fn allowed(world: &mut LibWorld) {
    ui(world, "pass_allow_nondeterministic");
}

#[then("it compiles")]
fn it_compiles(world: &mut LibWorld) {
    let case = world.notes["ui"].as_str().expect("a case").to_string();
    let dir = format!("{}/tests/rust/ui", env!("CARGO_MANIFEST_DIR"));
    assert!(std::path::Path::new(&format!("{dir}/{case}.rs")).exists());
    // trybuild's `pass` cases (verified by the `rust` test target) have no expected compiler output.
    assert!(!std::path::Path::new(&format!("{dir}/{case}.stderr")).exists());
}

#[given(regex = r#"^a completed instance of a workflow that calls "A" then "B"$"#)]
async fn completed_ab(world: &mut LibWorld) {
    super::common::durable(world).await;
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[when(regex = r#"^the code changes to call "B" then "A"$"#)]
fn code_reordered(world: &mut LibWorld) {
    world.only = Some(vec![super::variant("api.abc.reordered").register]);
}

#[then(
    regex = r#"^it returns one "Divergence" naming the instance and the first step that differs$"#
)]
fn one_divergence(world: &mut LibWorld) {
    let divergences = world.divergences.clone().expect("replayed");
    assert_eq!(divergences.len(), 1);
    assert_eq!(divergences[0].instance_id, world.runs[world.last()].id);
    assert_eq!(divergences[0].step, 0);
}

#[then("the divergence names the source location of the step")]
fn divergence_location(world: &mut LibWorld) {
    let divergence = world.divergences.clone().expect("replayed").remove(0);
    assert!(divergence.call_site.contains("api.rs:"), "{divergence:?}");
}

#[given(regex = r#"^a crate that depends on "jackdaw"$"#)]
fn a_crate(_world: &mut LibWorld) {}

#[when(regex = r#"^it contains only "use jackdaw::prelude::\*;"$"#)]
fn only_prelude(world: &mut LibWorld) {
    ui(world, "pass_prelude");
}

#[then("it can name each of:")]
fn names_each(world: &mut LibWorld, step: &GherkinStep) {
    let source = std::fs::read_to_string(format!(
        "{}/tests/rust/ui/pass_prelude.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the prelude case");
    assert!(source.contains("use jackdaw::prelude::*;"));
    let table = step.table.as_ref().expect("a table");
    for row in table.rows.iter().skip(1) {
        let name = &row[0];
        assert!(
            source.contains(name.as_str()),
            "the prelude case does not use `{name}`"
        );
    }
    let _ = world;
}

#[then(regex = r#"^"jackdaw::api::(\w+)" is marked "\#\[non_exhaustive\]"$"#)]
fn marked_non_exhaustive(world: &mut LibWorld, ty: String) {
    let dir = format!("{}/src/api", env!("CARGO_MANIFEST_DIR"));
    let found = std::fs::read_dir(&dir).expect("src/api").any(|entry| {
        let text = std::fs::read_to_string(entry.expect("an entry").path()).unwrap_or_default();
        text.contains(&format!("#[non_exhaustive]\npub enum {ty} "))
    });
    assert!(found, "{ty} is not #[non_exhaustive]");
    world.notes.insert(
        "ui".into(),
        json!(format!("misuse_exhaustive_{}", ty.to_lowercase())),
    );
}

#[then(
    regex = r#"^a downstream "match" on "jackdaw::api::\w+" without a wildcard arm fails to compile$"#
)]
fn downstream_match(world: &mut LibWorld) {
    assert!(stderr(world).contains("non-exhaustive patterns"));
}

// ---------------------------------------------------------------------------------------------------------------
// api/engine_builder.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(regex = r#"^an engine built with "Engine::builder\(\)\.build\(\)"$"#)]
fn memory_builder(world: &mut LibWorld) {
    world.durable = false;
}

#[when("I execute a workflow to completion")]
async fn execute_to_completion(world: &mut LibWorld) {
    let how = world
        .notes
        .get("cache-use")
        .cloned()
        .unwrap_or(json!(CacheUse::Cached));
    world
        .start_fixture("it", super::fixture("api.cache"), how)
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[when(
    regex = r"^I drop the engine and build a new one (?:the same way|on the same path|on the same store)$"
)]
async fn drop_and_rebuild(world: &mut LibWorld) {
    world.crash().await;
    world.engine();
}

#[then("the new engine reports the instance as unknown")]
async fn unknown(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    assert!(world.engine().status(&id).await.is_err());
}

#[given(regex = r#"^an engine built with a "Redb" store at a temporary path$"#)]
async fn redb_builder(world: &mut LibWorld) {
    super::common::durable(world).await;
}

#[given(
    regex = r#"^an engine built with "(Redb::open\(path\)|Sqlite::open\(path\)|Postgres::connect\(url\))"$"#
)]
async fn typed_store(world: &mut LibWorld, store: String) {
    let kind = store.split("::").next().unwrap_or_default().to_lowercase();
    world.store = Some(kind);
    super::common::durable(world).await;
}

#[then("a task result cached by the first engine is served from the cache by the second")]
async fn served_from_cache(world: &mut LibWorld) {
    world
        .start_fixture(
            "again",
            super::fixture("api.cache"),
            json!(CacheUse::Cached),
        )
        .await;
    let again = world.run_index("again");
    world.outcome(again).await.value();
    assert_eq!(world.probe.count("api_cached"), 1);
}

/// A third-party store: in-memory maps behind the Store trait.
#[derive(Clone)]
pub struct MapStore {
    pub persistence: Arc<jackdaw::providers::persistence::InMemoryPersistence>,
    pub cache: Arc<jackdaw::providers::cache::mem::InMemoryCache>,
}

impl Store for MapStore {
    fn persistence(&self) -> Arc<dyn jackdaw::persistence::PersistenceProvider> {
        Arc::clone(&self.persistence) as Arc<dyn jackdaw::persistence::PersistenceProvider>
    }
    fn cache(&self) -> Arc<dyn jackdaw::cache::CacheProvider> {
        Arc::clone(&self.cache) as Arc<dyn jackdaw::cache::CacheProvider>
    }
}

#[given(regex = r#"^a type that implements "jackdaw::api::Store" using in-memory maps$"#)]
fn map_store(world: &mut LibWorld) {
    let store = MapStore {
        persistence: Arc::new(jackdaw::providers::persistence::InMemoryPersistence::new()),
        cache: Arc::new(jackdaw::providers::cache::mem::InMemoryCache::new()),
    };
    world.extras.push(Arc::new(move |builder: EngineBuilder| {
        builder.store(store.clone())
    }));
}

#[when(regex = r#"^I pass it to "EngineBuilder::store"$"#)]
async fn pass_store(world: &mut LibWorld) {
    for label in ["first", "second"] {
        world
            .start_fixture(label, super::fixture("api.cache"), json!(CacheUse::Cached))
            .await;
        let index = world.run_index(label);
        world.outcome(index).await.value();
    }
}

#[then("the engine persists journals and cached results through it")]
async fn persists_through(world: &mut LibWorld) {
    let first = world.runs[world.run_index("first")].id.clone();
    let listed = world.engine().list(&ListFilter::new()).await.expect("list");
    assert!(listed.iter().any(|info| info.id == first));
    assert_eq!(
        world.probe.count("api_cached"),
        1,
        "the second run used the store's cache"
    );
}

#[given(regex = r#"^a "SqlitePersistence" and a "SqliteCache" from "jackdaw::providers"$"#)]
async fn raw_sqlite(world: &mut LibWorld) {
    let url = format!(
        "sqlite:{}?mode=rwc",
        world.dir.path().join("raw.sqlite").display()
    );
    let persistence = jackdaw::providers::persistence::SqlitePersistence::new(&url)
        .await
        .expect("persistence");
    let cache = jackdaw::providers::cache::SqliteCache::new(&url)
        .await
        .expect("cache");
    let (persistence, cache) = (Arc::new(persistence), Arc::new(cache));
    world.extras.push(Arc::new(move |builder: EngineBuilder| {
        builder
            .persistence(Arc::clone(&persistence) as _)
            .cache(Arc::clone(&cache) as _)
    }));
}

#[when(regex = r#"^I pass them to "EngineBuilder::persistence" and "EngineBuilder::cache"$"#)]
fn pass_providers(world: &mut LibWorld) {
    world.build_error = world.builder().build().err().map(|e| e.to_string());
}

#[then("the engine builds")]
fn engine_builds(world: &mut LibWorld) {
    assert_eq!(world.build_error, None);
}

#[given(regex = r#"^a workflow whose "\#\[run\]" calls the task "Charge"$"#)]
fn run_calls_charge(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.charge-only"));
}

#[then(regex = r#"^"build" fails with "Error::Invalid"$"#)]
fn build_invalid(world: &mut LibWorld) {
    let error = world.build_error.clone().expect("the build failed");
    assert!(error.starts_with("Invalid"), "{error}");
}

#[then(regex = r#"^the message names "(\w+)"$"#)]
fn message_names(world: &mut LibWorld, name: String) {
    let error = world.build_error.clone().expect("the build failed");
    assert!(
        error.to_lowercase().contains(&name.to_lowercase()),
        "{error}"
    );
}

#[given(regex = r#"^a workflow whose "\#\[run\]" runs the child workflow "Payment"$"#)]
fn runs_payment(world: &mut LibWorld) {
    world.set_fixture(super::fixture("kids.order"));
}

#[when("I build an engine that registers only the parent")]
fn only_parent(world: &mut LibWorld) {
    world.only = Some(vec![super::fixture("kids.order").register]);
    world.build_error = world.builder().build().err().map(|e| e.to_string());
}

#[then("the engine builds and can run the parent with its child")]
async fn runs_parent_child(world: &mut LibWorld) {
    assert_eq!(world.build_error, None);
    world
        .start_fixture("it", super::fixture("kids.order"), json!(500))
        .await;
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({ "charge": "ch-500" })
    );
}

#[jackdaw::prelude::catalog(source = "file://./tests/rust/fixtures/catalog")]
pub trait MissingEntry {
    #[task(name = "add-numbers", version = "9.9.9")]
    async fn add(input: Value) -> Result<Value, TaskError>;
}

#[given(regex = r#"^a "\#\[catalog\]" that imports a task the catalog does not contain$"#)]
fn missing_catalog(_world: &mut LibWorld) {}

#[when("I build an engine with that catalog")]
fn build_with_catalog(world: &mut LibWorld) {
    world.build_error = world
        .builder()
        .catalog::<MissingEntry>()
        .build()
        .err()
        .map(|e| format!("{e:?}"));
}

#[given(regex = r#"^an engine built with "event_buffer_size\(1\)"$"#)]
fn buffer_one(world: &mut LibWorld) {
    world.extras.push(Arc::new(|builder: EngineBuilder| {
        builder.event_buffer_size(1)
    }));
}

#[when(regex = r"^a workflow emits 10 events and nobody reads them$")]
async fn emits_ten(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.four-tasks"), json!(null))
        .await;
}

#[then(regex = r"^a subscriber that joins after completion receives all (\d+) events in order$")]
async fn late_subscriber(world: &mut LibWorld, n: usize) {
    let last = world.last();
    world.outcome(last).await.value();
    let mut events = world.handle(last).events();
    let mut names = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.name());
    }
    assert_eq!(names.len(), n, "{names:?}");
    assert_eq!(names.first(), Some(&"WorkflowStarted"));
    assert_eq!(names.last(), Some(&"WorkflowCompleted"));
}

// ---------------------------------------------------------------------------------------------------------------
// api/events.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(regex = r#"^a workflow that calls "ctx\.wait_for_event::<Payment>\(\\"paid\\"\)"$"#)]
fn waits_payment(world: &mut LibWorld) {
    world.set_fixture(super::fixture("events.paid"));
}

#[when(regex = r#"^I call "send_event\(\\"paid\\", Payment \{ cents: (\d+) \}\)" on its handle$"#)]
async fn send_payment(world: &mut LibWorld, cents: u64) {
    world.start("it", json!(null)).await;
    world
        .handle(world.last())
        .send_event("paid", json!({ "cents": cents }))
        .await
        .expect("send");
}

#[then(regex = r"^the step returns the payment with (\d+) cents$")]
async fn payment_cents(world: &mut LibWorld, cents: u64) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(cents));
}

#[given(regex = r#"^a workflow that sleeps 1 second and then waits for "paid"$"#)]
fn second_then_wait(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.second-then-wait"));
}

#[when(regex = r#"^I send "paid" immediately after starting it$"#)]
async fn send_immediately(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    world
        .handle(world.last())
        .send_event("paid", json!({ "cents": 9 }))
        .await
        .expect("send");
}

#[then("the workflow receives the event once it starts waiting")]
async fn receives_once_waiting(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(9));
}

#[when(regex = r#"^I send "paid" to the running instance$"#)]
async fn send_running(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    world.quiesce().await;
    world
        .handle(world.last())
        .send_event("paid", json!({ "cents": 11 }))
        .await
        .expect("send");
}

#[when(regex = r"^the virtual clock advances (\d+ \w+)$")]
async fn virtual_clock(world: &mut LibWorld, span: String) {
    world.pass(duration(&span)).await;
}

#[then("the workflow receives the event without it being sent again")]
async fn receives_without_resend(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(11));
}

#[given(regex = r"^an engine with a store that records every write$")]
fn records_every_write(world: &mut LibWorld) {
    super::durability::record_writes(world);
}

#[when(regex = r#"^"send_event" returns successfully$"#)]
async fn send_returns(world: &mut LibWorld) {
    world.probe.script("work", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("events.work-then-wait"), json!(null))
        .await;
    world
        .handle(world.last())
        .send_event("paid", json!({ "cents": 13 }))
        .await
        .expect("send");
}

#[then("the store has recorded the event for that instance")]
fn store_recorded(world: &mut LibWorld) {
    assert!(super::durability::store_contains(world, "\"Delivered\""));
    world.probe.release_all();
}

#[given("a workflow with two task steps")]
fn two_steps(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.chain"));
}

#[when(regex = r#"^I execute it and read "ExecutionHandle::events" to the end$"#)]
async fn read_events(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
    let last = world.last();
    world.outcome(last).await.value();
    let mut events = world.handle(last).events();
    let mut names = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.name());
    }
    world.notes.insert("events".into(), json!(names));
    world
        .notes
        .insert("after-end".into(), json!(events.next().await.is_none()));
}

#[then("the event names are, in order:")]
fn event_names(world: &mut LibWorld, step: &GherkinStep) {
    let table = step.table.as_ref().expect("a table");
    let expected: Vec<Value> = table.rows.iter().skip(1).map(|row| json!(row[0])).collect();
    assert_eq!(world.notes["events"], json!(expected));
}

#[then(regex = r#"^the stream ends with "None" after the terminal event$"#)]
fn stream_ends(world: &mut LibWorld) {
    assert_eq!(world.notes["after-end"], json!(true));
}

#[given("an instance in a Redb store that completed two steps before a crash")]
async fn two_then_crash(world: &mut LibWorld) {
    super::common::durable(world).await;
    world.probe.script("c", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world.until("c runs", || probe.running("c")).await;
    world.crash().await;
    world.probe.release_all();
}

#[when("a new engine resumes it")]
async fn resumes_it(world: &mut LibWorld) {
    let last = world.last();
    world.resume(last).await;
    world.outcome(last).await.value();
}

#[then(
    regex = r#"^every "TaskCompleted" event replayed from the journal reports "replayed\(\) == true"$"#
)]
async fn replayed_true(world: &mut LibWorld) {
    let mut events = world.handle(world.last()).events();
    let mut completed = Vec::new();
    while let Some(event) = events.next().await {
        if let Event::TaskCompleted {
            task_name,
            replayed,
            ..
        } = &event
        {
            completed.push((task_name.clone(), *replayed));
        }
    }
    assert_eq!(
        completed[..2],
        [("dur.a".to_string(), true), ("dur.b".to_string(), true)]
    );
    world.notes.insert(
        "live".into(),
        json!(completed.get(2).map(|(_, replayed)| *replayed)),
    );
}

#[then(regex = r#"^the step that runs live reports "replayed\(\) == false"$"#)]
fn live_false(world: &mut LibWorld) {
    assert_eq!(world.notes["live"], json!(false));
}

#[given("a completed instance in a Redb store")]
async fn completed_redb(world: &mut LibWorld) {
    super::common::durable(world).await;
    world
        .start_fixture("it", super::fixture("calls.chain"), json!("A-1"))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
    let mut events = world.handle(last).events();
    let mut names = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.name());
    }
    world.notes.insert("streamed".into(), json!(names));
}

#[when(regex = r#"^I call "Engine::history" with its id from a new engine$"#)]
async fn history_new_engine(world: &mut LibWorld) {
    world.crash().await;
    let id = world.runs[world.last()].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    world.notes.insert(
        "history".into(),
        json!(history.iter().map(Event::name).collect::<Vec<_>>()),
    );
}

#[then("I get the same events the stream delivered")]
fn same_events(world: &mut LibWorld) {
    assert_eq!(world.notes["history"], world.notes["streamed"]);
}

// ---------------------------------------------------------------------------------------------------------------
// api/signals_queries_updates.feature
// ---------------------------------------------------------------------------------------------------------------

#[given(regex = r#"^the "Approval" workflow whose state has an "approved" flag$"#)]
fn approval_workflow(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.approval"));
}

#[given(regex = r#"^a "\#\[signal\] approve" handler that sets the flag$"#)]
fn approve_handler(_world: &mut LibWorld) {}

#[given(regex = r#"^a "\#\[query\] is_approved" handler that returns the flag$"#)]
fn is_approved_handler(_world: &mut LibWorld) {}

#[given(
    regex = r#"^a "\#\[run\]" body that waits with "ctx\.wait_condition\(\|s\| s\.approved\)"$"#
)]
fn waits_with(_world: &mut LibWorld) {}

#[given(regex = r#"^a running "Approval" instance$"#)]
async fn running_approval(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.approval"), json!(null))
        .await;
    world.quiesce().await;
}

#[when(regex = r#"^I send the signal "approve"(?: through its handle)?$"#)]
async fn send_approve(world: &mut LibWorld) {
    world
        .handle(world.last())
        .signal(Approval::approve, ())
        .await
        .expect("signal");
}

#[when(regex = r#"^I query "is_approved"$"#)]
async fn query_approved(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let updated = world
        .engine()
        .describe(&id)
        .await
        .expect("describe")
        .updated_at;
    world.notes.insert("updated".into(), json!(updated));
    let answer = world
        .handle(world.last())
        .query(Approval::is_approved)
        .await
        .expect("query");
    world.notes.insert("answer".into(), json!(answer));
}

#[then(regex = r"^the answer is (true|false)$")]
fn answer_bool(world: &mut LibWorld, answer: String) {
    assert_eq!(world.notes["answer"], json!(answer == "true"));
}

#[then("the instance's journal has no new entries")]
async fn no_new_entries(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let updated = world
        .engine()
        .describe(&id)
        .await
        .expect("describe")
        .updated_at;
    assert_eq!(json!(updated), world.notes["updated"]);
}

#[when(regex = r#"^I send "approve" with an argument of the wrong type$"#)]
fn wrong_argument(world: &mut LibWorld) {
    ui(world, "misuse_signal_argument");
}

#[then("it fails to compile")]
fn fails_to_compile(world: &mut LibWorld) {
    assert!(stderr(world).contains("E0308"));
}

#[when("the engine is dropped before the instance takes its next step")]
async fn dropped_before_next(world: &mut LibWorld) {
    world.crash().await;
}

#[then("the instance completes without the signal being sent again")]
async fn completes_without_resend(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("approved"));
}

#[given(regex = r#"^a completed "Approval" instance$"#)]
async fn completed_approval(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.approval"), json!(null))
        .await;
    world
        .handle(world.last())
        .signal(Approval::approve, ())
        .await
        .expect("signal");
    let last = world.last();
    world.outcome(last).await.value();
}

#[when(regex = r#"^I send it the signal "approve"$"#)]
async fn send_to_finished(world: &mut LibWorld) {
    let result = world
        .handle(world.last())
        .signal(Approval::approve, ())
        .await;
    world.notes.insert(
        "signal_error".into(),
        json!(result.err().map(|e| e.to_string())),
    );
}

#[then("the call fails with an error that says the instance is not running")]
fn not_running(world: &mut LibWorld) {
    let error = world.notes["signal_error"]
        .as_str()
        .expect("the call failed");
    assert!(error.contains("not running"), "{error}");
}

#[given(regex = r#"^a "\#\[update\] set_limit\(.+" handler that returns the previous limit$"#)]
fn set_limit_handler(world: &mut LibWorld) {
    world.set_fixture(super::fixture("api.limits"));
}

#[given(regex = r"^a running instance with limit (\d+)$")]
async fn running_limit(world: &mut LibWorld, _limit: u64) {
    world
        .start_fixture("it", super::fixture("api.limits"), json!(null))
        .await;
    world.quiesce().await;
}

async fn send_update(world: &mut LibWorld, limit: u64, id: Option<&str>) {
    let handle = world.handle(world.last());
    let result = match id {
        Some(id) => handle.update_with_id(Limits::set_limit, limit, id).await,
        None => handle.update(Limits::set_limit, limit).await,
    };
    let entry = match result {
        Ok(previous) => json!({ "ok": previous }),
        Err(error) => json!({ "err": error.to_string() }),
    };
    world
        .notes
        .entry("updates".into())
        .or_insert(json!([]))
        .as_array_mut()
        .expect("a list")
        .push(entry);
}

#[when(regex = r#"^I send the update "set_limit" with (\d+)$"#)]
async fn update_limit(world: &mut LibWorld, limit: u64) {
    if world.runs.is_empty() {
        running_limit(world, 10).await;
    }
    send_update(world, limit, None).await;
}

#[then(regex = r"^the caller receives (\d+)$")]
fn caller_receives(world: &mut LibWorld, n: u64) {
    assert_eq!(world.notes["updates"][0], json!({ "ok": n }));
}

#[then(regex = r"^a query of the limit returns (\d+)$")]
async fn query_limit(world: &mut LibWorld, n: u64) {
    let (limit, _) = world
        .handle(world.last())
        .query(Limits::limit)
        .await
        .expect("query");
    assert_eq!(limit, n);
}

#[given("an update handler that rejects limits above 100")]
async fn rejecting_handler(world: &mut LibWorld) {
    running_limit(world, 10).await;
}

#[then("the caller receives the rejection")]
fn receives_rejection(world: &mut LibWorld) {
    let error = world.notes["updates"][0]["err"].as_str().expect("rejected");
    assert!(error.contains("above 100"), "{error}");
}

#[then("the workflow's state is unchanged")]
async fn state_unchanged(world: &mut LibWorld) {
    let state = world
        .handle(world.last())
        .query(Limits::limit)
        .await
        .expect("query");
    assert_eq!(state, (10, 0));
}

#[then("the journal records no update")]
async fn no_update_recorded(world: &mut LibWorld) {
    world.crash().await;
    let last = world.last();
    world.resume(last).await;
    let state = world
        .handle(last)
        .query(Limits::limit)
        .await
        .expect("query");
    assert_eq!(state, (10, 0));
}

#[given(regex = r#"^an instance that accepted the update "set_limit" with (\d+)$"#)]
async fn accepted_update(world: &mut LibWorld, limit: u64) {
    super::common::durable(world).await;
    running_limit(world, 10).await;
    send_update(world, limit, None).await;
}

#[then("the update handler runs once in the replay at the same point")]
async fn handler_once(world: &mut LibWorld) {
    let state = world
        .handle(world.last())
        .query(Limits::limit)
        .await
        .expect("query");
    assert_eq!(state, (20, 1));
}

#[then("the caller is not sent a second result")]
fn no_second_result(world: &mut LibWorld) {
    assert_eq!(world.notes["updates"].as_array().map(Vec::len), Some(1));
}

#[when(regex = r#"^I send the update "set_limit" with (\d+) and update id "([\w-]+)" twice$"#)]
async fn update_twice(world: &mut LibWorld, limit: u64, id: String) {
    running_limit(world, 10).await;
    send_update(world, limit, Some(&id)).await;
    send_update(world, limit, Some(&id)).await;
}

#[then("the handler runs once")]
async fn handler_runs_once(world: &mut LibWorld) {
    let state = world
        .handle(world.last())
        .query(Limits::limit)
        .await
        .expect("query");
    assert_eq!(state.1, 1);
}

#[then("both calls receive the same result")]
fn same_result(world: &mut LibWorld) {
    assert_eq!(world.notes["updates"][0], world.notes["updates"][1]);
}

// ---------------------------------------------------------------------------------------------------------------
// api/public_surface.feature: the curated surface
// ---------------------------------------------------------------------------------------------------------------

#[then("it can name each of these from the crate root:")]
fn names_from_root(_world: &mut LibWorld, step: &GherkinStep) {
    // `pass_crate_root.rs` is compiled by the trybuild `pass_*` cases.
    let source = std::fs::read_to_string(format!(
        "{}/tests/rust/ui/pass_crate_root.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the crate-root case");
    let imports = source
        .lines()
        .find(|line| line.starts_with("use jackdaw::{"))
        .expect("a crate-root import");
    for row in step.table.as_ref().expect("a table").rows.iter().skip(1) {
        let name = &row[0];
        assert!(
            imports.contains(&format!("{name},")) || imports.contains(&format!("{name}}}")),
            "{name} is not imported from the crate root"
        );
    }
}

/// `cargo doc` for jackdaw with warnings denied, run once per test binary: `(succeeded, stderr)`.
async fn documented() -> &'static (bool, String) {
    static DOCS: tokio::sync::OnceCell<(bool, String)> = tokio::sync::OnceCell::const_new();
    DOCS.get_or_init(|| async {
        let output =
            tokio::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
                .args(["doc", "-q", "-p", "jackdaw", "--all-features", "--no-deps"])
                .env("RUSTDOCFLAGS", "-D warnings")
                .output()
                .await
                .expect("cargo runs");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    })
    .await
}

fn doc_dir() -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").map_or_else(
        |_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"),
        Into::into,
    );
    target.join("doc/jackdaw")
}

#[when(regex = r#"^I build the docs for "jackdaw"$"#)]
async fn build_docs(_world: &mut LibWorld) {
    let (ok, stderr) = documented().await;
    assert!(ok, "{stderr}");
}

#[when(
    regex = r#"^I run "cargo doc -p jackdaw --all-features --no-deps" with "RUSTDOCFLAGS=-D warnings"$"#
)]
async fn doc_with_warnings_denied(world: &mut LibWorld) {
    let (ok, stderr) = documented().await;
    world
        .notes
        .insert("doc".into(), json!({ "ok": ok, "stderr": stderr }));
}

#[then("the command succeeds")]
fn command_succeeds(world: &mut LibWorld) {
    let doc = &world.notes["doc"];
    assert_eq!(doc["ok"], json!(true), "{}", doc["stderr"]);
}

#[then(regex = r#"^"src/lib\.rs" contains "\#!\[warn\(missing_docs\)\]"$"#)]
fn lib_warns_missing_docs(_world: &mut LibWorld) {
    let lib = std::fs::read_to_string(format!("{}/src/lib.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("lib.rs");
    assert!(lib.contains("#![warn(missing_docs)]"));
}

#[then(
    regex = r#"^the items used only by generated macro code are under a "\#\[doc\(hidden\)\]" module named "__private"$"#
)]
fn private_module(_world: &mut LibWorld) {
    let lib = std::fs::read_to_string(format!("{}/src/lib.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("lib.rs");
    assert!(
        lib.contains("#[doc(hidden)]\npub mod __private {"),
        "no hidden __private module"
    );
    assert!(
        !doc_dir().join("__private/index.html").exists(),
        "__private has a docs page"
    );
}

#[then(regex = r#"^the "api" module page does not list any of:$"#)]
fn api_page_omits(_world: &mut LibWorld, step: &GherkinStep) {
    let page =
        std::fs::read_to_string(doc_dir().join("api/index.html")).expect("the api docs page");
    for row in step.table.as_ref().expect("a table").rows.iter().skip(1) {
        let name = &row[0];
        let linked = ["struct", "trait", "fn", "type", "enum"]
            .iter()
            .any(|kind| page.contains(&format!("{kind}.{name}.html")));
        assert!(!linked, "the api page lists {name}");
    }
}

#[then(regex = r#"^the macros still expand and compile against "::jackdaw::__private"$"#)]
fn macros_use_private(_world: &mut LibWorld) {
    let dir = format!("{}/../jackdaw-macros/src", env!("CARGO_MANIFEST_DIR"));
    let sources: String = std::fs::read_dir(dir)
        .expect("the macro sources")
        .map(|entry| std::fs::read_to_string(entry.expect("an entry").path()).unwrap_or_default())
        .collect();
    assert!(sources.contains("::jackdaw::__private::"));
    assert!(
        !sources.contains("::jackdaw::api::"),
        "the macros still emit `::jackdaw::api` paths"
    );
    // Every fixture of this suite is expanded by those macros, and the suite compiled.
}

#[then(
    regex = r#"^"jackdaw::(persistence|cache|executor)::Error" is marked "\#\[non_exhaustive\]"$"#
)]
fn extension_error_non_exhaustive(_world: &mut LibWorld, module: String) {
    let source = std::fs::read_to_string(format!("{}/src/{module}.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("the module");
    assert!(
        source.contains("#[non_exhaustive]\npub enum Error {"),
        "{module}::Error is not #[non_exhaustive]"
    );
    let stderr = std::fs::read_to_string(format!(
        "{}/tests/rust/ui/misuse_exhaustive_{module}_error.stderr",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the compile-fail case");
    assert!(stderr.contains("non-exhaustive patterns"));
}

#[given(regex = r#"^a crate that depends on "jackdaw" with all features$"#)]
fn a_crate_all_features(_world: &mut LibWorld) {}

#[when(regex = r#"^it refers to "jackdaw::(\w+)(?:::\w+)*"$"#)]
fn refers_to(world: &mut LibWorld, module: String) {
    // `misuse_private_<module>.rs` names the path; trybuild's `misuse_*` cases compile it and check the error.
    ui(world, &format!("misuse_private_{module}"));
}

#[then("it fails to compile because the item is private")]
fn fails_private(world: &mut LibWorld) {
    let stderr = stderr(world);
    assert!(stderr.contains("is private"), "{stderr}");
}
