//! tasks/recorded_values.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub struct Recorded(pub Arc<Probe>);

#[tasks]
impl Recorded {
    #[task(name = "rec.keep")]
    pub async fn keep(self: Arc<Self>, ctx: TaskContext, value: Value) -> Result<Value, TaskError> {
        self.0.typed("keep", &ctx, &value).await
    }
    #[task(name = "rec.audit")]
    pub async fn audit(self: Arc<Self>, ctx: TaskContext, order: u32) -> Result<(), TaskError> {
        self.0.typed("audit", &ctx, &order).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("audit", |_| json!(null));
}

#[workflow(namespace = "rec", name = "read", version = "1")]
pub struct ReadValue;

#[workflow_methods]
impl ReadValue {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, which: String) -> WorkflowResult<Value> {
        let value = match which.as_str() {
            "ctx.now()" => json!(ctx.now()),
            "ctx.random()" => json!(ctx.random()),
            "ctx.uuid()" => json!(ctx.uuid()),
            other => json!(format!("unknown {other}")),
        };
        Ok(ctx.call(Recorded::keep, &value).await?)
    }
}

#[workflow(namespace = "rec", name = "audit", version = "1")]
pub struct MaybeAudit;

#[workflow_methods]
impl MaybeAudit {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: u32) -> WorkflowResult<bool> {
        let audited = ctx.random() < 0.1;
        if audited {
            ctx.call(Recorded::audit, &order).await?;
        }
        Ok(audited)
    }
}

#[workflow(namespace = "rec", name = "clock", version = "1")]
pub struct ClockReads;

#[workflow_methods]
impl ClockReads {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<i64> {
        let before = ctx.now();
        ctx.sleep(1.hours()).await?;
        let after = ctx.now();
        Ok((after - before).num_seconds())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<ReadValue>("rec.read"),
        Fixture::of::<MaybeAudit>("rec.audit").input(json!(1)),
        Fixture::of::<ClockReads>("rec.clock"),
    ]
}

#[given(regex = r#"^a workflow that reads "(ctx\.\w+\(\))" and passes it to a task$"#)]
fn reads(world: &mut LibWorld, call: String) {
    world.set_fixture(super::fixture("rec.read").input(json!(call)));
}

#[when("I execute it and replay it")]
async fn execute_and_replay(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
    let last = world.last();
    let first = world.outcome(last).await.value().clone();
    world.notes.insert("first".into(), first);
    world.clock.advance(Duration::from_secs(3600));
    super::common::replay_all(world).await;
}

#[then("the replay passes the same value to the task")]
fn same_value(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    assert_eq!(
        world.probe.count("keep"),
        1,
        "the replay did not run the task again"
    );
    assert_eq!(world.probe.calls_of("keep")[0].input, world.notes["first"]);
}

#[then(regex = r#"^"Engine::replay_all" reports no divergence$"#)]
async fn replay_all_clean(world: &mut LibWorld) {
    if world.divergences.is_none() {
        let engine = world.engine();
        world.divergences = Some(engine.replay_all().await.expect("replay_all"));
    }
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
}

#[given(
    regex = r#"^a workflow that does "if ctx\.random\(\) < 0\.1 \{ ctx\.call\(Audit, &order\)\.await\?; \}"$"#
)]
fn maybe_audit(world: &mut LibWorld) {
    world.set_fixture(super::fixture("rec.audit"));
}

#[when(regex = r"^I execute (\d+) instances and replay each one$")]
async fn execute_many(world: &mut LibWorld, n: u32) {
    for order in 0..n {
        world
            .start_fixture(
                &format!("i{order}"),
                super::fixture("rec.audit"),
                json!(order),
            )
            .await;
    }
    world.settle().await;
    super::common::replay_all(world).await;
}

#[then("every replay makes the same audit decision as its first run")]
fn same_decisions(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    let audited = world
        .runs
        .iter()
        .filter(|run| {
            run.outcome
                .as_ref()
                .is_some_and(|o| o.value() == &json!(true))
        })
        .count();
    assert_eq!(world.probe.count("audit"), audited);
}

#[given(
    regex = r#"^a workflow that reads "ctx\.now\(\)" before and after a 1-hour sleep on a virtual clock$"#
)]
fn clock_reads(world: &mut LibWorld) {
    world.set_fixture(super::fixture("rec.clock"));
}

#[then(regex = r"^the two values are (\d+) hours? apart$")]
async fn apart(world: &mut LibWorld, hours: i64) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(hours * 3600));
}
