//! tasks/waiting_for_events.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Paid {
    pub cents: u64,
}

pub struct Waits(pub Arc<Probe>);

#[tasks]
impl Waits {
    #[task(name = "events.work")]
    pub async fn work(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("work", &ctx, &()).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("work", |_| json!(null));
}

#[workflow(namespace = "events", name = "paid", version = "1")]
pub struct WaitPaid;

#[workflow_methods]
impl WaitPaid {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        let paid: Paid = ctx.wait_for_event("paid").await?;
        Ok(paid.cents)
    }
}

#[workflow(namespace = "events", name = "work-then-wait", version = "1")]
pub struct WorkThenWait;

#[workflow_methods]
impl WorkThenWait {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        ctx.call(Waits::work, &()).await?;
        let paid: Paid = ctx.wait_for_event("paid").await?;
        Ok(paid.cents)
    }
}

#[workflow(namespace = "events", name = "lines", version = "1")]
pub struct Lines;

#[workflow_methods]
impl Lines {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<u32>> {
        let mut got = Vec::new();
        for _ in 0..3 {
            got.push(ctx.wait_for_event::<u32>("line").await?);
        }
        Ok(got)
    }
}

#[workflow(namespace = "events", name = "two-names", version = "1")]
pub struct TwoNames;

#[workflow_methods]
impl TwoNames {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<String>> {
        let shipped: String = ctx.wait_for_event("shipped").await?;
        let paid: String = ctx.wait_for_event("paid").await?;
        Ok(vec![shipped, paid])
    }
}

#[workflow(namespace = "events", name = "sleep-then-wait", version = "1")]
pub struct SleepThenWait;

#[workflow_methods]
impl SleepThenWait {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        ctx.sleep(1.hours()).await?;
        let paid: Paid = ctx.wait_for_event("paid").await?;
        Ok(paid.cents)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<WaitPaid>("events.paid"),
        Fixture::of::<WorkThenWait>("events.work-then-wait"),
        Fixture::of::<Lines>("events.lines"),
        Fixture::of::<TwoNames>("events.two-names"),
        Fixture::of::<SleepThenWait>("events.sleep-then-wait"),
    ]
}

async fn send(world: &mut LibWorld, name: &str, payload: Value) {
    let last = world.last();
    world
        .handle(last)
        .send_event(name, payload)
        .await
        .expect("send_event");
}

#[given(
    regex = r#"^a workflow that does "let paid: Paid = ctx\.wait_for_event\(\\"paid\\"\)\.await\?"$"#
)]
fn waits_paid(world: &mut LibWorld) {
    world.set_fixture(super::fixture("events.paid"));
}

#[when(regex = r#"^I call "handle\.send_event\(\\"paid\\", Paid \{ cents: (\d+) \}\)"$"#)]
async fn send_paid(world: &mut LibWorld, cents: u64) {
    world.start("it", json!(null)).await;
    send(world, "paid", json!({ "cents": cents })).await;
}

#[then(regex = r"^the workflow continues with (\d+) cents$")]
async fn continues_with(world: &mut LibWorld, cents: u64) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(cents));
}

#[given(regex = r#"^a workflow that runs a 1-second task and then waits for "paid"$"#)]
fn work_then_wait(world: &mut LibWorld) {
    world
        .probe
        .script("work", |s| s.delay = Some(Duration::from_secs(1)));
    world.set_fixture(super::fixture("events.work-then-wait"));
}

#[when(regex = r#"^"paid" is sent while the task is still running$"#)]
async fn sent_while_running(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    let probe = Arc::clone(&world.probe);
    world
        .until_frozen("the task runs", || probe.running("work"))
        .await;
    send(world, "paid", json!({ "cents": 300 })).await;
    assert!(world.probe.running("work"));
}

#[then("the workflow receives it as soon as it starts waiting")]
async fn receives_when_waiting(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(300));
}

#[given(regex = r#"^a workflow that waits for "line" three times in a loop$"#)]
fn lines(world: &mut LibWorld) {
    world.set_fixture(super::fixture("events.lines"));
}

#[when(regex = r#"^"line" is sent with 1, 2 and 3$"#)]
async fn send_lines(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    for n in 1..=3 {
        send(world, "line", json!(n)).await;
    }
}

#[then("the workflow receives 1, 2 and 3 in that order")]
async fn in_order(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!([1, 2, 3]));
}

#[given(regex = r#"^a workflow that waits for "shipped" and then for "paid"$"#)]
fn two_names(world: &mut LibWorld) {
    world.set_fixture(super::fixture("events.two-names"));
}

#[when(regex = r#"^"paid" is sent before "shipped"$"#)]
async fn paid_first(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    send(world, "paid", json!("paid!")).await;
    send(world, "shipped", json!("shipped!")).await;
}

#[then("the workflow receives both in its own order")]
async fn own_order(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!(["shipped!", "paid!"])
    );
}

#[given(regex = r#"^a workflow that waits for "paid" as "Paid \{ cents: u64 \}"$"#)]
fn waits_typed(world: &mut LibWorld) {
    world.set_fixture(super::fixture("events.paid"));
}

#[when(regex = r#"^"paid" is sent with '(.+)'$"#)]
async fn sent_with(world: &mut LibWorld, payload: String) {
    world.start("it", json!(null)).await;
    send(world, "paid", serde_json::from_str(&payload).expect("JSON")).await;
}

#[then(regex = r#"^the step fails with a validation fault naming "(\w+)"$"#)]
async fn validation_naming(world: &mut LibWorld, name: String) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.kind, error_types::VALIDATION);
    assert!(
        fault
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains(&format!("`{name}`")),
        "{fault:?}"
    );
}

#[given(regex = r#"^a completed instance that received "paid"$"#)]
async fn completed_paid(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("events.paid"), json!(null))
        .await;
    send(world, "paid", json!({ "cents": 500 })).await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[then("the replay receives the same payload without the event being sent")]
fn replay_same_payload(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
}

#[given(regex = r#"^a workflow that sleeps 1 hour and then waits for "paid"$"#)]
fn sleeps_then_waits(world: &mut LibWorld) {
    world.set_fixture(super::fixture("events.sleep-then-wait"));
}

#[when(regex = r#"^"paid" is sent and the engine is dropped during the sleep$"#)]
async fn sent_and_dropped(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    world.quiesce().await;
    send(world, "paid", json!({ "cents": 700 })).await;
    world.crash().await;
}

#[then(regex = r#"^the workflow receives "paid" without it being sent again$"#)]
async fn receives_without_resend(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(700));
}

#[given(regex = r#"^a running instance "([\w-]+)" waiting for "paid"$"#)]
async fn running_by_id(world: &mut LibWorld, id: String) {
    let fixture = super::fixture("events.paid");
    let engine = world.engine();
    let handle = (fixture.start_with_id)(engine, id, json!(null))
        .await
        .expect("starts");
    let now = world.clock.now();
    world.push_run("it", fixture, handle, now);
}

#[when(
    regex = r#"^another part of the program calls "Engine::handle::<W>\(\\"([\w-]+)\\"\)\?\.send_event\(\\"paid\\", \.\.\)"$"#
)]
async fn handle_by_id_sends(world: &mut LibWorld, id: String) {
    let engine = world.engine();
    let fixture = super::fixture("events.paid");
    let handle = (fixture.attach)(engine, id).await.expect("a handle by id");
    handle
        .send_event("paid", json!({ "cents": 42 }))
        .await
        .expect("send");
}

#[then("the instance receives it")]
async fn instance_receives(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(42));
}
