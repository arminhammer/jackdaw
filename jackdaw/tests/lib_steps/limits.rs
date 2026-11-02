//! tasks/task_limits.feature
//!
//! The tasks here coordinate through a [`Gate`] that belongs to one scenario. Its key travels in the task input, as
//! free-function tasks have no state of their own and scenarios run at the same time.
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How long a call waits for its partner or its release before failing, so a broken build fails instead of hanging.
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Counts {
    /// Rendezvous calls that have arrived.
    arrived: usize,
    /// Held calls running now, and the most that ever ran at once.
    running: usize,
    most: usize,
    /// Held calls that ran, counting each attempt.
    attempts: usize,
    released: bool,
}

#[derive(Default)]
pub struct Gate {
    counts: Mutex<Counts>,
    changed: Condvar,
}

impl Gate {
    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Blocks the calling thread until a second call arrives.
    fn rendezvous(&self) -> Result<(), TaskError> {
        let mut counts = self.counts();
        counts.arrived += 1;
        self.changed.notify_all();
        let (counts, timeout) = self
            .changed
            .wait_timeout_while(counts, PATIENCE, |counts| counts.arrived < 2)
            .unwrap_or_else(PoisonError::into_inner);
        drop(counts);
        if timeout.timed_out() {
            return Err(TaskError::non_retryable(
                "the second call never arrived: the calls did not run in parallel",
            ));
        }
        Ok(())
    }

    fn enter(&self) {
        let mut counts = self.counts();
        counts.running += 1;
        counts.attempts += 1;
        counts.most = counts.most.max(counts.running);
    }

    fn leave(&self) {
        self.counts().running -= 1;
    }

    /// Holds the calling thread until the scenario releases the calls.
    fn hold_blocking(&self) -> Result<(), TaskError> {
        self.enter();
        let counts = self.counts();
        let (counts, timeout) = self
            .changed
            .wait_timeout_while(counts, PATIENCE, |counts| !counts.released)
            .unwrap_or_else(PoisonError::into_inner);
        drop(counts);
        self.leave();
        if timeout.timed_out() {
            return Err(TaskError::non_retryable("the call was never released"));
        }
        Ok(())
    }

    /// Holds without blocking a thread until the scenario releases the calls.
    async fn hold(&self, ctx: &TaskContext) -> Result<(), TaskError> {
        self.enter();
        ctx.heartbeat(());
        let started = Instant::now();
        while !self.counts().released {
            if started.elapsed() > PATIENCE {
                self.leave();
                return Err(TaskError::non_retryable("the call was never released"));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.leave();
        Ok(())
    }

    fn release(&self) {
        self.counts().released = true;
        self.changed.notify_all();
    }
}

static GATES: LazyLock<Mutex<HashMap<String, Arc<Gate>>>> = LazyLock::new(Mutex::default);

fn gate(key: &str) -> Arc<Gate> {
    Arc::clone(
        GATES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.to_string())
            .or_default(),
    )
}

/// This scenario's gate key, made on first use.
fn key(world: &mut LibWorld) -> String {
    world
        .notes
        .entry("gate".into())
        .or_insert_with(|| json!(uuid::Uuid::new_v4().to_string()))
        .as_str()
        .expect("a gate key")
        .to_string()
}

fn scenario_gate(world: &mut LibWorld) -> Arc<Gate> {
    gate(&key(world))
}

// ---- tasks and workflows ----

/// An async task that blocks its thread: it stands in for CPU-bound work.
#[task(name = "lim.rendezvous")]
pub async fn rendezvous(key: String) -> Result<(), TaskError> {
    gate(&key).rendezvous()
}

#[workflow(namespace = "lim", name = "join-two", version = "1")]
pub struct JoinTwo;

#[workflow_methods]
impl JoinTwo {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, key: String) -> WorkflowResult<()> {
        futures::try_join!(ctx.call(Rendezvous, &key), ctx.call(Rendezvous, &key))?;
        Ok(())
    }
}

#[workflow(namespace = "lim", name = "each-two", version = "1")]
pub struct EachTwo;

#[workflow_methods]
impl EachTwo {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, key: String) -> WorkflowResult<()> {
        let keys = [key.clone(), key];
        ctx.for_each(&keys, Rendezvous).concurrency(2).await?;
        Ok(())
    }
}

/// Registers only this scenario's tasks and workflows, plus `extra` builder settings.
fn only(
    world: &mut LibWorld,
    register: impl Fn(EngineBuilder) -> EngineBuilder + Send + Sync + 'static,
) {
    world.no_tasks = true;
    world.only = Some(Vec::new());
    world.extras.push(Arc::new(register));
}

/// The input of the held tasks (a struct, so YAML can pass it `with:` a map).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonSchema)]
pub struct Held {
    pub key: String,
}

#[task(name = "lim.hold", limit = "render")]
pub async fn hold(ctx: TaskContext, held: Held) -> Result<(), TaskError> {
    gate(&held.key).hold(&ctx).await
}

#[task(name = "lim.free", limit = "shared")]
pub async fn free(ctx: TaskContext, held: Held) -> Result<(), TaskError> {
    gate(&held.key).hold(&ctx).await
}

pub struct Methods;

#[tasks]
impl Methods {
    #[task(name = "lim.method", limit = "shared")]
    pub async fn method(self: Arc<Self>, ctx: TaskContext, held: Held) -> Result<(), TaskError> {
        gate(&held.key).hold(&ctx).await
    }
}

#[task(name = "lim.hold-blocking", blocking)]
pub fn hold_blocking(held: Held) -> Result<(), TaskError> {
    gate(&held.key).hold_blocking()
}

#[task(name = "lim.hold-cpu", blocking, limit = "cpu")]
pub fn hold_cpu(held: Held) -> Result<(), TaskError> {
    gate(&held.key).hold_blocking()
}

#[workflow(namespace = "lim", name = "call-hold", version = "1")]
pub struct CallHold;

#[workflow_methods]
impl CallHold {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, held: Held) -> WorkflowResult<()> {
        Ok(ctx.call(Hold, &held).await?)
    }
}

#[workflow(namespace = "lim", name = "call-free", version = "1")]
pub struct CallFree;

#[workflow_methods]
impl CallFree {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, held: Held) -> WorkflowResult<()> {
        Ok(ctx.call(Free, &held).await?)
    }
}

#[workflow(namespace = "lim", name = "call-method", version = "1")]
pub struct CallMethod;

#[workflow_methods]
impl CallMethod {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, held: Held) -> WorkflowResult<()> {
        Ok(ctx.call(Methods::method, &held).await?)
    }
}

#[workflow(namespace = "lim", name = "call-blocking", version = "1")]
pub struct CallBlocking;

#[workflow_methods]
impl CallBlocking {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, held: Held) -> WorkflowResult<()> {
        Ok(ctx.call(HoldBlocking, &held).await?)
    }
}

#[workflow(namespace = "lim", name = "call-cpu", version = "1")]
pub struct CallCpu;

#[workflow_methods]
impl CallCpu {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, held: Held) -> WorkflowResult<()> {
        Ok(ctx.call(HoldCpu, &held).await?)
    }
}

#[workflow(namespace = "lim", name = "call-patient", version = "1")]
pub struct CallPatient;

#[workflow_methods]
impl CallPatient {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, held: Held) -> WorkflowResult<()> {
        Ok(ctx.call(Hold, &held).heartbeat_timeout(1.minutes()).await?)
    }
}

// ---- parallel calls ----

#[given("an async task that blocks its thread until a second call arrives")]
fn rendezvous_task(world: &mut LibWorld) {
    only(world, |builder| {
        builder
            .task(Rendezvous)
            .workflow::<JoinTwo>()
            .workflow::<EachTwo>()
    });
}

#[when(regex = r#"^a workflow calls it twice with "(.+)"$"#)]
async fn calls_twice(world: &mut LibWorld, fan_out: String) {
    let fixture = match fan_out.as_str() {
        "futures::try_join!" => Fixture::of::<JoinTwo>("lim.join-two"),
        "ctx.for_each(..).concurrency(2)" => Fixture::of::<EachTwo>("lim.each-two"),
        other => panic!("unknown fan-out {other}"),
    };
    let key = key(world);
    world.start_fixture("rendezvous", fixture, json!(key)).await;
}

#[then("both rendezvous calls complete")]
async fn both_complete(world: &mut LibWorld) {
    let run = world.run_index("rendezvous");
    let outcome = world.outcome_within(run, PATIENCE * 2).await;
    assert_eq!(outcome.value(), &Value::Null, "{outcome:?}");
}

// ---- limits ----

/// The workflow that calls the scenario's held task.
fn held_fixture(world: &LibWorld) -> Fixture {
    match world.notes["held"].as_str().expect("a held task") {
        "hold" => Fixture::of::<CallHold>("lim.call-hold"),
        "blocking" => Fixture::of::<CallBlocking>("lim.call-blocking"),
        "cpu" => Fixture::of::<CallCpu>("lim.call-cpu"),
        other => panic!("unknown held task {other}"),
    }
}

fn held_input(world: &mut LibWorld) -> Value {
    json!({ "key": key(world) })
}

#[given(
    regex = r##"^an async task marked "\#\[task\(limit = \\"render\\"\)\]" that holds until released$"##
)]
fn hold_task(world: &mut LibWorld) {
    world.notes.insert("held".into(), json!("hold"));
    only(world, |builder| {
        builder
            .task(Hold)
            .workflow::<CallHold>()
            .workflow::<CallPatient>()
    });
}

#[given(
    regex = r##"^a free-function task and a "\#\[tasks\]" method that both name the limit "shared" and hold until released$"##
)]
fn shared_tasks(world: &mut LibWorld) {
    only(world, |builder| {
        builder
            .task(Free)
            .tasks(Methods)
            .workflow::<CallFree>()
            .workflow::<CallMethod>()
    });
}

#[given(regex = r##"^a "\#\[task\(blocking\)\]" task that holds until released$"##)]
fn blocking_task(world: &mut LibWorld) {
    world.notes.insert("held".into(), json!("blocking"));
    only(world, |builder| {
        builder.task(HoldBlocking).workflow::<CallBlocking>()
    });
}

#[given(
    regex = r##"^a "\#\[task\(blocking, limit = \\"cpu\\"\)\]" task that holds until released$"##
)]
fn cpu_task(world: &mut LibWorld) {
    world.notes.insert("held".into(), json!("cpu"));
    only(world, |builder| builder.task(HoldCpu).workflow::<CallCpu>());
}

#[given(regex = r#"^the engine defines the limit "(\w+)" with (\d+) permits?$"#)]
fn defines_limit(world: &mut LibWorld, name: String, permits: usize) {
    world.extras.push(Arc::new(move |builder| {
        builder.limit(name.clone(), permits)
    }));
}

#[given(regex = r"^the engine sets a blocking limit of (\d+)$")]
fn sets_blocking_limit(world: &mut LibWorld, permits: usize) {
    world
        .extras
        .push(Arc::new(move |builder| builder.blocking_limit(permits)));
}

#[when(regex = r"^(\d+) workflows each call the held task$")]
async fn each_call(world: &mut LibWorld, count: usize) {
    let fixture = held_fixture(world);
    for i in 0..count {
        let input = held_input(world);
        world
            .start_fixture(&format!("held-{i}"), fixture.clone(), input)
            .await;
    }
}

#[when("one workflow calls the free-function task and another calls the method")]
async fn free_and_method(world: &mut LibWorld) {
    let input = held_input(world);
    world
        .start_fixture(
            "free",
            Fixture::of::<CallFree>("lim.call-free"),
            input.clone(),
        )
        .await;
    world
        .start_fixture(
            "method",
            Fixture::of::<CallMethod>("lim.call-method"),
            input,
        )
        .await;
}

#[given("one workflow's call holds the only permit")]
async fn first_holds(world: &mut LibWorld) {
    let input = held_input(world);
    let fixture = held_fixture(world);
    world.start_fixture("first", fixture, input).await;
    let gate = scenario_gate(world);
    world
        .until_frozen("the first call runs", || gate.counts().running == 1)
        .await;
}

#[when("another workflow calls the held task with a heartbeat timeout of 1 minute")]
async fn patient_call(world: &mut LibWorld) {
    let input = held_input(world);
    world
        .start_fixture(
            "patient",
            Fixture::of::<CallPatient>("lim.call-patient"),
            input,
        )
        .await;
}

#[when(regex = r"^(\d+) minutes pass while it waits for the permit$")]
async fn minutes_pass(world: &mut LibWorld, minutes: u64) {
    world.pass(Duration::from_secs(minutes * 60)).await;
    assert_eq!(
        scenario_gate(world).counts().running,
        1,
        "the second call waits"
    );
}

#[then(regex = r"^exactly (\d+) held calls? (?:is|are) running$")]
async fn exactly_running(world: &mut LibWorld, count: usize) {
    let gate = scenario_gate(world);
    world
        .until_frozen(&format!("{count} held calls run"), || {
            gate.counts().running >= count
        })
        .await;
    // Long enough for a call the limit should hold back to start, if it was going to.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let counts = gate.counts();
    assert_eq!((counts.running, counts.most), (count, count));
}

#[when("the held calls are released")]
fn release(world: &mut LibWorld) {
    scenario_gate(world).release();
}

#[then(regex = r"^all (\d+) workflows complete$")]
async fn all_complete(world: &mut LibWorld, count: usize) {
    assert_eq!(world.runs.len(), count);
    for run in 0..count {
        let outcome = world.outcome_within(run, PATIENCE * 2).await;
        assert_eq!(outcome.value(), &Value::Null, "{outcome:?}");
    }
}

#[then(regex = r"^at most (\d+) held calls? ever ran at once$")]
fn at_most(world: &mut LibWorld, count: usize) {
    let most = scenario_gate(world).counts().most;
    assert!(most <= count, "{most} calls ran at once");
}

#[then(regex = r"^no held calls? was retried$")]
fn not_retried(world: &mut LibWorld) {
    let attempts = scenario_gate(world).counts().attempts;
    assert_eq!(attempts, world.runs.len(), "each call ran once");
}

#[when(regex = r"^I build the engine (?:without defining the limit|with its limits)$")]
fn build(world: &mut LibWorld) {
    world.build_error = world.builder().build().err().map(|e| e.to_string());
}

#[then(regex = r#"^the build fails with "(.+)"$"#)]
fn build_fails(world: &mut LibWorld, message: String) {
    let error = world.build_error.clone().expect("the build failed");
    assert!(error.contains(&message), "{error}");
}

const FORK_HOLD: &str = r#"
document:
  dsl: '1.0.2'
  namespace: lim
  name: fork-hold
  version: '1'
do:
  - fan:
      fork:
        compete: false
        branches:
          - a:
              call: lim.hold
              with: { key: '${ .key }' }
          - b:
              call: lim.hold
              with: { key: '${ .key }' }
          - c:
              call: lim.hold
              with: { key: '${ .key }' }
"#;

#[given("a YAML workflow that forks 3 branches, each calling the held task")]
fn fork_hold(world: &mut LibWorld) {
    let workflow = Workflow::<Value, Value>::from_yaml(FORK_HOLD).expect("parses");
    world
        .extras
        .push(Arc::new(move |builder| builder.yaml_workflow(&workflow)));
}

#[when("I execute the forking YAML workflow")]
async fn execute_yaml(world: &mut LibWorld) {
    let workflow = Workflow::<Value, Value>::from_yaml(FORK_HOLD).expect("parses");
    let input = held_input(world);
    let started_virtual = world.clock.now();
    let handle = world
        .engine()
        .execute(&workflow, input)
        .await
        .expect("the YAML workflow starts");
    world.runs.push(super::world::Run {
        label: "yaml".into(),
        id: handle.id().to_string(),
        fixture: None,
        handle: Some(handle),
        outcome: None,
        finished_at: None,
        started_virtual,
    });
}

#[then("the YAML workflow completes")]
async fn yaml_completes(world: &mut LibWorld) {
    let run = world.run_index("yaml");
    let outcome = world.outcome_within(run, PATIENCE * 2).await;
    assert!(
        matches!(outcome, super::support::Outcome::Ok(_)),
        "{outcome:?}"
    );
}
