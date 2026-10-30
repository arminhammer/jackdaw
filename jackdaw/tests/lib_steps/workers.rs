//! operations/multi_worker.feature: engines that share a store with leases.
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub struct Work(pub Arc<Probe>);

#[tasks]
impl Work {
    #[task(name = "w.step")]
    pub async fn step(self: Arc<Self>, ctx: TaskContext, n: u32) -> Result<u32, TaskError> {
        self.0.typed(&format!("w_step{n}"), &ctx, &n).await
    }
}

pub fn defaults(probe: &Probe) {
    for n in 1..=3 {
        probe.default_output(&format!("w_step{n}"), |input| input.clone());
    }
}

#[workflow(namespace = "workers", name = "three-steps", version = "1")]
pub struct ThreeSteps;

#[workflow_methods]
impl ThreeSteps {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        let mut total = 0;
        for n in 1..=3 {
            total += ctx.call(Work::step, n).await?;
        }
        Ok(total)
    }
}

#[workflow(namespace = "workers", name = "approval", version = "1")]
pub struct Approval {
    answer: Option<String>,
}

#[workflow_methods]
impl Approval {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { answer: None }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        ctx.wait_condition(|approval| approval.answer.is_some())
            .await?;
        Ok(ctx.state(|approval| approval.answer.clone().unwrap_or_default()))
    }

    #[signal]
    fn answer(&mut self, _ctx: &mut SyncWorkflowContext<Self>, answer: String) {
        self.answer = Some(answer);
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<ThreeSteps>("workers.three-steps"),
        Fixture::of::<Approval>("workers.approval"),
    ]
}

const LEASE: Duration = Duration::from_secs(30);

/// The engine called `name`: `worker-1` is the scenario's engine, the others are built on the same store.
fn worker(world: &mut LibWorld, name: &str) -> Engine {
    if name == "worker-1" {
        return world.engine();
    }
    if let Some(engine) = world.workers.get(name) {
        return engine.clone();
    }
    let engine = world
        .builder()
        .engine_id(name)
        .build()
        .expect("the worker builds");
    world.workers.insert(name.to_string(), engine.clone());
    engine
}

/// Starts `o-1` on worker-1 with `fixture`.
async fn start_o1(world: &mut LibWorld, fixture: &str) -> usize {
    let fixture = super::fixture(fixture);
    let engine = world.engine();
    let started = world.clock.now();
    let handle = (fixture.start_with_id)(engine, "o-1".into(), Value::Null)
        .await
        .expect("o-1 starts");
    world.push_run("o-1", fixture, handle, started)
}

/// Starts `o-1` on worker-1 and waits until it is running step `n` (which blocks).
async fn running_step(world: &mut LibWorld, n: u32) -> usize {
    world
        .probe
        .script(&format!("w_step{n}"), |s| s.block = true);
    let run = start_o1(world, "workers.three-steps").await;
    let probe = Arc::clone(&world.probe);
    let name = format!("w_step{n}");
    world
        .until("o-1 runs its step", || probe.running(&name))
        .await;
    run
}

#[given(regex = r#"^a store shared by engine processes "worker-1" and "worker-2"$"#)]
async fn shared_store(world: &mut LibWorld) {
    super::common::durable(world).await;
    if world.store_kind() == "redb" {
        // One process cannot open a Redb file twice, so the "processes" share one handle to it.
        let path = world.dir.path().join("jackdaw.redb");
        let store =
            jackdaw::providers::persistence::RedbPersistence::new(&path.display().to_string())
                .expect("the store opens");
        world.persistence = Some(Arc::new(store));
    }
    world.extras.push(Arc::new(|builder: EngineBuilder| {
        builder
            .lease(LEASE)
            .poll_interval(Duration::from_millis(20))
            .engine_id("worker-1")
    }));
}

#[given("both processes register the same workflows")]
fn same_workflows(_world: &mut LibWorld) {
    // Every engine of a scenario registers every fixture.
}

#[given(regex = r#"^"worker-1" is running instance "o-1"$"#)]
async fn worker1_running(world: &mut LibWorld) {
    running_step(world, 1).await;
}

#[when(regex = r#"^"worker-2" calls "Engine::recover\(\)"$"#)]
async fn worker2_recovers(world: &mut LibWorld) {
    let recovery = worker(world, "worker-2").recover().await.expect("recover");
    world
        .notes
        .insert("resumed".into(), json!(recovery.resumed));
}

#[then(regex = r#"^"worker-2" does not resume "o-1"$"#)]
fn not_resumed(world: &mut LibWorld) {
    assert_eq!(world.notes["resumed"], json!([]));
}

#[then(regex = r#"^every task of "o-1" runs once$"#)]
async fn every_task_once(world: &mut LibWorld) {
    world.probe.release("w_step1");
    let run = world.run_index("o-1");
    assert_eq!(world.outcome(run).await.value(), &json!(6));
    for n in 1..=3 {
        assert_eq!(world.probe.count(&format!("w_step{n}")), 1, "step {n}");
    }
}

#[given(regex = r#"^"worker-1" owns "o-1" with a lease of 30 seconds$"#)]
async fn owns_with_lease(world: &mut LibWorld) {
    running_step(world, 2).await;
}

#[when(regex = r#"^"worker-1" stops heartbeating$"#)]
async fn stops_heartbeating(world: &mut LibWorld) {
    world.crash().await;
    world.probe.script("w_step2", |s| s.block = false);
}

#[when(regex = r"^(\d+) seconds pass$")]
fn seconds_pass(world: &mut LibWorld, seconds: u64) {
    world.clock.advance(Duration::from_secs(seconds));
}

#[then(regex = r#"^"worker-2" takes over "o-1"$"#)]
fn takes_over(world: &mut LibWorld) {
    assert_eq!(world.notes["resumed"], json!(["o-1"]));
}

#[then(regex = r#"^"o-1" completes$"#)]
async fn o1_completes(world: &mut LibWorld) {
    let engine = worker(world, "worker-2");
    let output = engine
        .handle::<ThreeSteps>("o-1")
        .await
        .expect("a handle")
        .await
        .expect("o-1 completes");
    assert_eq!(output, 6);
}

#[then(regex = r#"^completed tasks of "o-1" are not run again$"#)]
fn not_run_again(world: &mut LibWorld) {
    assert_eq!(world.probe.completed("w_step1"), 1);
    assert_eq!(world.probe.completed("w_step3"), 1);
}

#[given(regex = r#"^"worker-1" owned "o-1" and was paused longer than its lease$"#)]
async fn paused_too_long(world: &mut LibWorld) {
    running_step(world, 2).await;
    world.clock.advance(LEASE + Duration::from_secs(1));
}

#[given(regex = r#"^"worker-2" has taken over "o-1"$"#)]
async fn has_taken_over(world: &mut LibWorld) {
    let recovery = worker(world, "worker-2").recover().await.expect("recover");
    assert_eq!(recovery.resumed, vec!["o-1".to_string()]);
}

#[when(regex = r#"^"worker-1" wakes up and tries to record a step for "o-1"$"#)]
async fn worker1_wakes(world: &mut LibWorld) {
    world.probe.script("w_step2", |s| s.block = false);
    world.probe.release("w_step2");
}

#[then(regex = r#"^the write is rejected because the lease is held by "worker-2"$"#)]
async fn write_rejected(world: &mut LibWorld) {
    let run = world.run_index("o-1");
    let message = world.outcome(run).await.message();
    assert!(message.contains("lost its lease"), "{message}");
}

#[then(regex = r#"^"worker-1" stops running "o-1"$"#)]
async fn worker1_stops(world: &mut LibWorld) {
    // Worker-2 finishes the instance; worker-1 wrote nothing after losing the lease, so the history is whole.
    let engine = worker(world, "worker-2");
    let output = engine
        .handle::<ThreeSteps>("o-1")
        .await
        .expect("a handle")
        .await
        .expect("o-1 completes");
    assert_eq!(output, 6);
    assert_eq!(world.probe.completed("w_step1"), 1);
    assert_eq!(world.probe.completed("w_step3"), 1);
    let status = world.engine().status("o-1").await.expect("a status");
    assert_eq!(status, Status::Completed);
}

#[given(regex = r#"^"worker-1" owns "o-1"$"#)]
async fn owns(world: &mut LibWorld) {
    start_o1(world, "workers.approval").await;
    world.quiesce().await;
}

#[when(regex = r#"^a signal for "o-1" is sent through "worker-2"$"#)]
async fn signal_through_worker2(world: &mut LibWorld) {
    let engine = worker(world, "worker-2");
    let handle = engine.handle::<Approval>("o-1").await.expect("a handle");
    handle
        .signal(Approval::answer, "approved".to_string())
        .await
        .expect("the signal is sent");
    let remote = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("the result reaches worker-2 through the store")
        .expect("o-1 completes");
    world.notes.insert("remote".into(), json!(remote));
}

#[then(regex = r#"^"o-1" receives the signal$"#)]
async fn receives_signal(world: &mut LibWorld) {
    let run = world.run_index("o-1");
    assert_eq!(world.outcome(run).await.value(), &json!("approved"));
    assert_eq!(world.notes["remote"], json!("approved"));
}

#[given("a store backend that does not support leases")]
fn no_leases(world: &mut LibWorld) {
    world.extras.clear();
    world.persistence = Some(Arc::new(super::interop::MapStore::default()));
}

#[when("a second engine is built on it while the first is running")]
fn second_engine(world: &mut LibWorld) {
    world.engine();
    world.build_error = world.builder().build().err().map(|e| e.to_string());
}

#[then(regex = r#"^"build" fails with an error that says the store is already in use$"#)]
fn already_in_use(world: &mut LibWorld) {
    let error = world.build_error.clone().expect("the build failed");
    assert!(error.contains("already in use"), "{error}");
}
