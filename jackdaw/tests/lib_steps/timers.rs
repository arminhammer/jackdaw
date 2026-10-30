//! tasks/timers_and_timeouts.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use chrono::{DateTime, Utc};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Paid {
    pub cents: u64,
}

pub struct Timed(pub Arc<Probe>);

#[tasks]
impl Timed {
    #[task(name = "timers.first")]
    pub async fn first(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("first", &ctx, &()).await
    }
    #[task(name = "timers.second")]
    pub async fn second(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("second", &ctx, &()).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("first", |_| json!(null));
    probe.default_output("second", |_| json!(null));
}

#[workflow(namespace = "timers", name = "between", version = "1")]
pub struct Between;

#[workflow_methods]
impl Between {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.call(Timed::first, &()).await?;
        ctx.sleep(3.days()).await?;
        ctx.call(Timed::second, &()).await?;
        Ok(())
    }
}

#[workflow(namespace = "timers", name = "until", version = "1")]
pub struct Until;

#[workflow_methods]
impl Until {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, deadline: DateTime<Utc>) -> WorkflowResult<u64> {
        let wait = (deadline - ctx.now()).to_std().unwrap_or_default();
        ctx.sleep(wait).await?;
        Ok(wait.as_secs())
    }
}

#[workflow(namespace = "timers", name = "event-timeout", version = "1")]
pub struct EventTimeout;

#[workflow_methods]
impl EventTimeout {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(
            match ctx.wait_for_event::<Paid>("paid").timeout(1.hours()).await {
                Ok(paid) => format!("paid {}", paid.cents),
                Err(StepError::Timeout { .. }) => "Err(StepError::Timeout { .. })".into(),
                Err(other) => format!("other: {other}"),
            },
        )
    }
}

#[workflow(namespace = "timers", name = "slow", version = "1")]
pub struct Slow;

#[workflow_methods]
impl Slow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.sleep(10.minutes()).await?;
        Ok(())
    }
}

#[workflow(namespace = "timers", name = "child-timeout", version = "1")]
pub struct ChildTimeout;

#[workflow_methods]
impl ChildTimeout {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(match ctx.run(Slow::run, &()).timeout(5.minutes()).await {
            Ok(()) => "finished".into(),
            Err(StepError::Timeout { .. }) => "timed out".into(),
            Err(other) => format!("other: {other}"),
        })
    }
}

#[workflow(namespace = "timers", name = "nap", version = "1")]
pub struct Nap;

#[workflow_methods]
impl Nap {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, hours: u64) -> WorkflowResult<()> {
        ctx.sleep(Duration::from_secs(hours * 3600)).await?;
        Ok(())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Between>("timers.between"),
        Fixture::of::<Until>("timers.until"),
        Fixture::of::<EventTimeout>("timers.event-timeout"),
        Fixture::of::<Slow>("timers.slow"),
        Fixture::of::<ChildTimeout>("timers.child-timeout"),
        Fixture::of::<Nap>("timers.nap").input(json!(1)),
    ]
}

#[given(regex = r#"^a workflow that calls "ctx\.sleep\(3\.days\(\)\)" between two task calls$"#)]
fn sleep_between(world: &mut LibWorld) {
    world.set_fixture(super::fixture("timers.between"));
}

#[then(regex = r"^the second task has not run after (\d+) hours$")]
async fn not_after(world: &mut LibWorld, hours: u64) {
    world.pass(Duration::from_secs(hours * 3600)).await;
    assert_eq!(world.probe.count("first"), 1);
    assert_eq!(world.probe.count("second"), 0);
}

#[then(regex = r"^it has run after (\d+) hours$")]
async fn has_run_after(world: &mut LibWorld, hours: u64) {
    let target = Duration::from_secs(hours * 3600);
    let more = target.saturating_sub(world.clock.elapsed());
    world.pass(more).await;
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.count("second"), 1);
}

#[given(
    regex = r#"^a workflow that sleeps for the difference between an input deadline and "ctx\.now\(\)"$"#
)]
fn sleeps_until(world: &mut LibWorld) {
    world.set_fixture(super::fixture("timers.until"));
}

#[when(regex = r"^I execute it with a deadline (\d+) hours away$")]
async fn deadline_away(world: &mut LibWorld, hours: i64) {
    let deadline = world.clock.now() + chrono::Duration::hours(hours);
    world.start("it", json!(deadline)).await;
}

#[then(regex = r"^the workflow wakes after (\d+) hours$")]
async fn wakes_after(world: &mut LibWorld, hours: u64) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(hours * 3600));
    assert_eq!(world.clock.elapsed(), Duration::from_secs(hours * 3600));
}

#[then("a replay computes the same duration")]
async fn replay_same(world: &mut LibWorld) {
    world.clock.advance(Duration::from_secs(3600));
    super::common::replay_all(world).await;
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
}

#[given(
    regex = r#"^a workflow that does "ctx\.wait_for_event::<Paid>\(\\"paid\\"\)\.timeout\(1\.hours\(\)\)\.await"$"#
)]
fn event_with_timeout(world: &mut LibWorld) {
    world.set_fixture(super::fixture("timers.event-timeout"));
}

#[when(regex = r"^no event arrives for (\d+) hours?$")]
async fn no_event_for(world: &mut LibWorld, hours: u64) {
    world.start("it", json!(null)).await;
    world.pass(Duration::from_secs(hours * 3600)).await;
}

#[then(regex = r#"^the step returns "(.+)"$"#)]
async fn step_returns(world: &mut LibWorld, expected: String) {
    if let Some(output) = world.notes.get("te-output") {
        // A run on a TestEngine of its own (testing_engine.rs).
        assert_eq!(output, &json!(expected));
        return;
    }
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(expected));
}

#[given(
    regex = r#"^a workflow that does "ctx\.run\(Slow::run, &input\)\.timeout\(5\.minutes\(\)\)\.await"$"#
)]
fn child_timeout(world: &mut LibWorld) {
    world.set_fixture(super::fixture("timers.child-timeout"));
}

// `"Slow" takes 10 minutes` matches the generic `"<task>" takes <duration>` step in saga.rs; `Slow` sleeps 10 minutes.

#[then(regex = r"^the step times out after (\d+) minutes$")]
async fn times_out_after(world: &mut LibWorld, minutes: u64) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("timed out"));
    assert_eq!(world.clock.elapsed(), Duration::from_secs(minutes * 60));
}

#[given(regex = r#"^a "TestEngine" running a workflow that sleeps 30 days$"#)]
fn test_engine_sleeps(world: &mut LibWorld) {
    world.notes.insert("test-engine".into(), json!(true));
}

#[then("it completes in under a second of wall-clock time")]
fn under_a_second(world: &mut LibWorld) {
    let took = world.notes["wall"].as_f64().expect("timed");
    assert!(took < 1.0, "took {took}s");
    assert_eq!(world.notes["virtual_days"], json!(30));
}

/// Runs the TestEngine on its own paused runtime, the way `#[tokio::test(start_paused = true)]` would.
pub fn run_test_engine(world: &mut LibWorld) {
    let started = Instant::now();
    let elapsed = std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("a runtime");
        runtime.block_on(async {
            let engine = jackdaw::testing::TestEngine::builder()
                .workflow::<Nap>()
                .build()
                .expect("builds");
            engine
                .execute(Nap::run, 24 * 30)
                .await
                .expect("starts")
                .await
                .expect("finishes");
            engine.clock().elapsed()
        })
    })
    .join()
    .expect("the test engine thread");
    world
        .notes
        .insert("wall".into(), json!(started.elapsed().as_secs_f64()));
    world
        .notes
        .insert("virtual_days".into(), json!(elapsed.as_secs() / 86_400));
}

#[given(regex = r"^a workflow sleeping 1 hour that has slept (\d+) minutes$")]
async fn slept_minutes(world: &mut LibWorld, minutes: u64) {
    world
        .start_fixture("it", super::fixture("timers.nap"), json!(1))
        .await;
    world.pass(Duration::from_secs(minutes * 60)).await;
}

#[then(regex = r"^the workflow wakes (\d+) minutes later$")]
async fn wakes_later(world: &mut LibWorld, minutes: u64) {
    let before = world.clock.elapsed();
    let last = world.last();
    world.pass(Duration::from_secs(minutes * 60 - 1)).await;
    assert!(!world.ended(last), "woke too early");
    world.outcome(last).await.value();
    assert_eq!(
        world.clock.elapsed() - before,
        Duration::from_secs(minutes * 60)
    );
}

#[given(regex = r#"^a step with "\.timeout\(1\.hours\(\)\)" that has been waiting (\d+) minutes$"#)]
async fn timeout_waiting(world: &mut LibWorld, minutes: u64) {
    world
        .start_fixture("it", super::fixture("timers.event-timeout"), json!(null))
        .await;
    world.pass(Duration::from_secs(minutes * 60)).await;
}

#[then(regex = r"^the step times out (\d+) minutes later$")]
async fn times_out_later(world: &mut LibWorld, minutes: u64) {
    let before = world.clock.elapsed();
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!("Err(StepError::Timeout { .. })")
    );
    assert_eq!(
        world.clock.elapsed() - before,
        Duration::from_secs(minutes * 60)
    );
}

#[given(regex = r"^a workflow executed with an execution timeout of (\d+) days?$")]
async fn execution_timeout(world: &mut LibWorld, days: u64) {
    let engine = world.engine();
    let handle = engine
        .execute_with(
            Nap::run,
            24 * 7,
            jackdaw::api::ExecuteOptions::new().timeout(days.days()),
        )
        .await
        .expect("starts")
        .erase();
    let now = world.clock.now();
    world.push_run("it", super::fixture("timers.nap"), handle, now);
}

#[when(regex = r"^it is still running after (\d+) days?$")]
async fn still_running_after(world: &mut LibWorld, days: u64) {
    world.pass(Duration::from_secs(days * 86_400 - 1)).await;
    assert!(!world.ended(world.last()));
    world.pass(Duration::from_secs(1)).await;
}

#[then(regex = r#"^it ends with "Error::Timeout"$"#)]
async fn ends_with_timeout(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(matches!(outcome, Outcome::Timeout(_)), "{outcome:?}");
}

#[then(regex = r#"^its status is "(\w+)"$"#)]
async fn its_status(world: &mut LibWorld, status: String) {
    let id = world.runs[world.last()].id.clone();
    let actual = world.engine().status(&id).await.expect("status");
    assert_eq!(format!("{actual:?}"), status);
}
