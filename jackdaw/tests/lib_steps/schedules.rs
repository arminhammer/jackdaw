//! operations/schedules.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::api::{CatchUp, ListFilter, Overlap, Schedule};
use jackdaw::prelude::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub struct Scheduled(pub Arc<Probe>);

#[tasks]
impl Scheduled {
    #[task(name = "sched.tick")]
    pub async fn tick(self: Arc<Self>, ctx: TaskContext, what: String) -> Result<(), TaskError> {
        self.0.typed("sched_tick", &ctx, &what).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("sched_tick", |_| json!(null));
}

#[workflow(namespace = "sched", name = "report", version = "1")]
pub struct ReportWorkflow;

#[workflow_methods]
impl ReportWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, what: String) -> WorkflowResult<()> {
        Ok(ctx.call(Scheduled::tick, &what).await?)
    }
}

#[workflow(namespace = "sched", name = "slow", version = "1")]
pub struct SlowWorkflow;

#[workflow_methods]
impl SlowWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, what: String) -> WorkflowResult<()> {
        ctx.call(Scheduled::tick, &what).await?;
        ctx.sleep(3.minutes()).await?;
        Ok(())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<ReportWorkflow>("sched.report"),
        Fixture::of::<SlowWorkflow>("sched.slow"),
    ]
}

fn at(time: &str) -> chrono::DateTime<chrono::Utc> {
    let full = if time.len() == 5 {
        format!("2026-01-01T{time}:00Z")
    } else {
        time.to_string()
    };
    full.parse().expect("a time")
}

fn add(world: &mut LibWorld, schedule: Schedule) {
    world.extras.push(Arc::new(move |builder: EngineBuilder| {
        builder.schedule(schedule.clone())
    }));
}

/// Builds the engine (which starts its schedules) and lets them take their first look at the clock.
async fn start_engine(world: &mut LibWorld) {
    world.engine();
    world.quiesce_until(world.clock.now()).await;
}

/// The fire times of a schedule's instances (their ids carry them).
async fn instances(world: &mut LibWorld, schedule: &str) -> Vec<chrono::DateTime<chrono::Utc>> {
    let prefix = format!("{schedule}/");
    let mut starts: Vec<chrono::DateTime<chrono::Utc>> = world
        .engine()
        .list(&ListFilter::new())
        .await
        .expect("list")
        .into_iter()
        .filter_map(|instance| Some(instance.id.strip_prefix(&prefix)?.parse().expect("a time")))
        .collect();
    starts.sort();
    starts
}

fn fired(world: &LibWorld, what: &str) -> Vec<chrono::DateTime<chrono::Utc>> {
    world
        .probe
        .calls_of("sched_tick")
        .into_iter()
        .filter(|call| call.input == json!(what))
        .map(|call| call.started_at)
        .collect()
}

#[given(regex = r#"^a schedule "([\w-]+)" for "ReportWorkflow::run" with cron "(.+)"$"#)]
fn cron_schedule(world: &mut LibWorld, name: String, cron: String) {
    add(
        world,
        Schedule::cron(name.clone(), &cron, ReportWorkflow::run, name).expect("cron"),
    );
}

#[when(regex = r#"^the clock moves from "(.+)" to "(.+)"$"#)]
async fn clock_moves(world: &mut LibWorld, from: String, to: String) {
    assert_eq!(world.clock.now(), at(&from));
    start_engine(world).await;
    world
        .pass((at(&to) - at(&from)).to_std().expect("forward"))
        .await;
}

#[then(regex = r#"^"ReportWorkflow" was started twice$"#)]
async fn started_twice(world: &mut LibWorld) {
    assert_eq!(instances(world, "nightly-report").await.len(), 2);
    assert_eq!(fired(world, "nightly-report").len(), 2);
}

#[then(regex = r#"^the instances were started at "(.+)" and "(.+)"$"#)]
async fn started_at(world: &mut LibWorld, first: String, second: String) {
    assert_eq!(
        instances(world, "nightly-report").await,
        vec![at(&first), at(&second)]
    );
}

#[given(regex = r#"^a schedule for "PollWorkflow::run" every (\d+) minutes$"#)]
async fn interval(world: &mut LibWorld, minutes: u64) {
    add(
        world,
        Schedule::every(
            "poll",
            Duration::from_secs(minutes * 60),
            ReportWorkflow::run,
            "poll".to_string(),
        ),
    );
    start_engine(world).await;
}

#[then(regex = r#"^"PollWorkflow" was started (\d+) times$"#)]
fn poll_started(world: &mut LibWorld, n: usize) {
    assert_eq!(fired(world, "poll").len(), n);
}

#[given(regex = r#"^a schedule with cron "(.+)" and catch-up policy "(\w+)"$"#)]
async fn catch_up(world: &mut LibWorld, cron: String, policy: String) {
    let policy = match policy.as_str() {
        "one" => CatchUp::One,
        "all" => CatchUp::All,
        _ => CatchUp::Skip,
    };
    add(
        world,
        Schedule::cron("hourly", &cron, ReportWorkflow::run, "hourly".to_string())
            .expect("cron")
            .catch_up(policy),
    );
    start_engine(world).await;
}

#[given(regex = r#"^the engine was down from "(.+)" to "(.+)"$"#)]
async fn down(world: &mut LibWorld, from: String, to: String) {
    world
        .pass((at(&from) - world.clock.now()).to_std().expect("forward"))
        .await;
    world.crash().await;
    world.notes.insert("down-from".into(), json!(at(&from)));
    world.clock.advance_to(at(&to));
}

#[when(regex = r#"^a new engine starts at "(.+)"$"#)]
async fn new_engine_at(world: &mut LibWorld, time: String) {
    assert_eq!(world.clock.now(), at(&time));
    start_engine(world).await;
    world.quiesce_until(world.clock.now()).await;
}

#[then(regex = r"^(\d+) instances are started for the missed ticks$")]
fn missed(world: &mut LibWorld, n: usize) {
    let down: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(world.notes["down-from"].clone()).expect("a time");
    let after: Vec<_> = fired(world, "hourly")
        .into_iter()
        .filter(|t| *t > down)
        .collect();
    assert_eq!(after.len(), n, "{after:?}");
}

#[given(regex = r#"^a schedule that fired at "(.+)"$"#)]
async fn fired_at(world: &mut LibWorld, time: String) {
    add(
        world,
        Schedule::cron(
            "daily",
            "0 2 * * *",
            ReportWorkflow::run,
            "daily".to_string(),
        )
        .expect("cron"),
    );
    start_engine(world).await;
    world
        .pass((at(&time) - world.clock.now()).to_std().expect("forward"))
        .await;
    assert_eq!(fired(world, "daily").len(), 1);
}

#[when(regex = r#"^the engine restarts at "(.+)"$"#)]
async fn restarts_at(world: &mut LibWorld, time: String) {
    world
        .pass((at(&time) - world.clock.now()).to_std().expect("forward"))
        .await;
    world.crash().await;
    start_engine(world).await;
}

#[then(regex = r#"^the "(.+)" tick is not fired again$"#)]
fn not_again(world: &mut LibWorld, _time: String) {
    assert_eq!(fired(world, "daily").len(), 1);
}

#[given(regex = r"^a schedule every minute for a workflow that takes 3 minutes$")]
fn every_minute(_world: &mut LibWorld) {}

#[given(regex = r#"^the overlap policy is "(\w+)"$"#)]
async fn overlap_policy(world: &mut LibWorld, policy: String) {
    let overlap = if policy == "skip" {
        Overlap::Skip
    } else {
        Overlap::Allow
    };
    add(
        world,
        Schedule::every(
            "every-minute",
            Duration::from_secs(60),
            SlowWorkflow::run,
            "slow".to_string(),
        )
        .overlap(overlap),
    );
    start_engine(world).await;
}

#[then("no two instances of the schedule run at the same time")]
async fn no_overlap(world: &mut LibWorld) {
    // Each run takes 3 minutes; with overlap allowed a run would start every minute.
    let starts = instances(world, "every-minute").await;
    assert!(!starts.is_empty());
    for pair in starts.windows(2) {
        assert!(
            pair[1] - pair[0] >= chrono::Duration::minutes(3),
            "{starts:?}"
        );
    }
}

#[given(regex = r#"^a schedule "([\w-]+)"$"#)]
fn named_schedule(world: &mut LibWorld, name: String) {
    world.notes.insert("schedule".into(), json!(name));
}

#[when(regex = r#"^it fires for "(.+)" on two engines that share a store$"#)]
async fn two_engines(world: &mut LibWorld, time: String) {
    super::durability::record_writes(world);
    let name = world.notes["schedule"]
        .as_str()
        .expect("a name")
        .to_string();
    add(
        world,
        Schedule::cron(name.clone(), "0 2 * * *", ReportWorkflow::run, name).expect("cron"),
    );
    let second = world.builder().build().expect("a second engine");
    start_engine(world).await;
    world
        .pass((at(&time) - world.clock.now()).to_std().expect("forward"))
        .await;
    world.notes.insert("engines".into(), json!(2));
    drop(second);
}

#[then(regex = r#"^one instance "(.+)" exists$"#)]
async fn one_instance(world: &mut LibWorld, id: String) {
    let ids: Vec<String> = world
        .engine()
        .list(&ListFilter::new())
        .await
        .expect("list")
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ids, vec![id]);
    let name = world.notes["schedule"]
        .as_str()
        .expect("a name")
        .to_string();
    assert_eq!(fired(world, &name).len(), 1, "the workflow ran once");
}

#[given("a running schedule")]
async fn running_schedule(world: &mut LibWorld) {
    add(
        world,
        Schedule::every(
            "pausable",
            Duration::from_secs(600),
            ReportWorkflow::run,
            "pausable".to_string(),
        ),
    );
    start_engine(world).await;
}

#[when("I pause it")]
async fn pause(world: &mut LibWorld) {
    world
        .engine()
        .pause_schedule("pausable")
        .await
        .expect("pause");
}

#[then("no ticks fire until I resume it")]
async fn none_until_resume(world: &mut LibWorld) {
    world.pass(Duration::from_secs(3600)).await;
    assert_eq!(fired(world, "pausable").len(), 0);
    world
        .engine()
        .resume_schedule("pausable")
        .await
        .expect("resume");
    world.quiesce_until(world.clock.now()).await;
    world.pass(Duration::from_secs(600)).await;
    assert_eq!(fired(world, "pausable").len(), 1);
}

#[when("I remove it")]
async fn remove(world: &mut LibWorld) {
    world
        .engine()
        .remove_schedule("pausable")
        .await
        .expect("remove");
    world
        .notes
        .insert("before".into(), json!(fired(world, "pausable").len()));
}

#[then("no further ticks fire after a restart")]
async fn none_after_restart(world: &mut LibWorld) {
    world.crash().await;
    start_engine(world).await;
    world.pass(Duration::from_secs(3600)).await;
    assert_eq!(json!(fired(world, "pausable").len()), world.notes["before"]);
}
