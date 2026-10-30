//! tasks/cancellation.feature (and compensation on cancel from error_handling.feature)
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub struct Cancels(pub Arc<Probe>);

#[tasks]
impl Cancels {
    #[task(name = "cancel.next")]
    pub async fn next(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("next", &ctx, &()).await
    }
    #[task(name = "cancel.reserve")]
    pub async fn reserve(self: Arc<Self>, ctx: TaskContext, n: u32) -> Result<String, TaskError> {
        self.0.typed("reserve_hold", &ctx, &n).await
    }
    #[task(name = "cancel.release")]
    pub async fn release(self: Arc<Self>, ctx: TaskContext, hold: String) -> Result<(), TaskError> {
        self.0.typed("release_stock", &ctx, &hold).await
    }
    #[task(name = "cancel.wait")]
    pub async fn wait(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("wait", &ctx, &()).await
    }
    /// Loops until its instance is cancelled.
    #[task(name = "cancel.poll")]
    pub async fn poll(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        let probe = Arc::clone(&self.0);
        let record = probe.invoke("poll", &ctx, json!(null));
        tokio::select! {
            () = ctx.cancelled() => Err(TaskError::non_retryable("stopped: cancelled")),
            result = record => result.map(|_| ()),
        }
    }
}

pub fn defaults(probe: &Probe) {
    // `release_stock` shares its default (null) with errors.rs.
    for name in ["next", "wait"] {
        probe.default_output(name, |_| json!(null));
    }
    probe.default_output("reserve_hold", |n| json!(format!("hold-{n}")));
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, JsonSchema)]
pub enum Waiting {
    Sleep,
    Event,
    Condition,
}

#[workflow(namespace = "cancel", name = "waiting", version = "1")]
pub struct WaitingOn {
    never: bool,
}

#[workflow_methods]
impl WaitingOn {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { never: false }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, on: Waiting) -> WorkflowResult<()> {
        match on {
            Waiting::Sleep => ctx.sleep(1.hours()).await?,
            Waiting::Event => {
                let _: Value = ctx.wait_for_event("paid").await?;
            }
            Waiting::Condition => ctx.wait_condition(|w| w.never).await?,
        }
        ctx.call(Cancels::next, &()).await?;
        Ok(())
    }
}

#[workflow(namespace = "cancel", name = "clean-up", version = "1")]
pub struct CleanUp;

#[workflow_methods]
impl CleanUp {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        let hold = ctx.call(Cancels::reserve, &1).await?;
        match ctx.call(Cancels::wait, &()).await {
            Err(StepError::Cancelled) => {
                ctx.call(Cancels::release, &hold).shielded().await?;
                Err(StepError::Cancelled)
            }
            other => other,
        }
    }
}

#[workflow(namespace = "cancel", name = "compensating", version = "1")]
pub struct Compensating;

#[workflow_methods]
impl Compensating {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        let mut holds = Vec::new();
        for n in 1..=3u32 {
            match ctx.call(Cancels::reserve, &n).await {
                Ok(hold) => holds.push(hold),
                Err(StepError::Cancelled) => {
                    for hold in holds.iter().rev() {
                        ctx.call(Cancels::release, hold).shielded().await?;
                    }
                    return Err(StepError::Cancelled);
                }
                Err(other) => return Err(other),
            }
        }
        Ok(())
    }
}

#[workflow(namespace = "cancel", name = "polling", version = "1")]
pub struct Polling;

#[workflow_methods]
impl Polling {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        Ok(ctx.call(Cancels::poll, &()).shielded().await?)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<WaitingOn>("cancel.waiting"),
        Fixture::of::<CleanUp>("cancel.clean-up"),
        Fixture::of::<Compensating>("cancel.compensating"),
        Fixture::of::<Polling>("cancel.polling"),
    ]
}

#[given(regex = r#"^a workflow that is waiting on (.+)$"#)]
async fn waiting_on(world: &mut LibWorld, what: String) {
    let on = if what.starts_with("ctx.sleep") {
        Waiting::Sleep
    } else if what.starts_with("ctx.wait_for_event") {
        Waiting::Event
    } else {
        Waiting::Condition
    };
    world
        .start_fixture("it", super::fixture("cancel.waiting"), json!(on))
        .await;
    world.quiesce().await;
}

#[when(regex = r#"^I call "cancel" on its handle$"#)]
async fn cancel_handle(world: &mut LibWorld) {
    world.handle(world.last()).cancel().await.expect("cancel");
}

#[then(regex = r#"^awaiting the handle returns "Error::Cancelled"$"#)]
async fn returns_cancelled(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(matches!(outcome, Outcome::Cancelled), "{outcome:?}");
}

#[then(regex = r#"^the status is "(\w+)"$"#)]
async fn status_is(world: &mut LibWorld, status: String) {
    let id = world.runs[world.last()].id.clone();
    let actual = world.engine().status(&id).await.expect("status");
    assert_eq!(format!("{actual:?}"), status);
}

#[then(regex = r"^no step after .+ runs$")]
fn nothing_after(world: &mut LibWorld) {
    assert_eq!(world.probe.count("next"), 0);
}

#[given(
    regex = r#"^a workflow that matches "Err\(StepError::Cancelled\)" on its current step and calls "ReleaseStock" in a non-cancellable scope$"#
)]
async fn clean_up(world: &mut LibWorld) {
    world.probe.script("wait", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("cancel.clean-up"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the step waits", || probe.running("wait"))
        .await;
}

#[when("it is cancelled")]
async fn it_is_cancelled(world: &mut LibWorld) {
    world.handle(world.last()).cancel().await.expect("cancel");
}

#[then(regex = r#"^"release_stock" runs to completion$"#)]
async fn release_runs(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await;
    assert_eq!(world.probe.completed("release_stock"), 1);
}

#[then(regex = r#"^the instance ends "(\w+)"$"#)]
async fn instance_ends(world: &mut LibWorld, status: String) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(matches!(outcome, Outcome::Cancelled), "{outcome:?}");
    status_is(world, status).await;
}

#[given(regex = r#"^a task that loops until "TaskContext::cancelled\(\)" resolves$"#)]
async fn polling_task(world: &mut LibWorld) {
    world.probe.script("poll", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("cancel.polling"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world.until("the task runs", || probe.running("poll")).await;
}

#[when("its instance is cancelled")]
async fn its_instance_cancelled(world: &mut LibWorld) {
    world.notes.insert(
        "cancelled_at".into(),
        json!(std::time::Instant::now().elapsed().as_millis()),
    );
    world.handle(world.last()).cancel().await.expect("cancel");
}

#[then(regex = r"^the task stops within (\d+) seconds?$")]
async fn stops_within(world: &mut LibWorld, seconds: u64) {
    let started = std::time::Instant::now();
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(
        started.elapsed() < Duration::from_secs(seconds),
        "took {:?}",
        started.elapsed()
    );
    assert!(
        matches!(&outcome, Outcome::Faulted(f) if f.detail.as_deref() == Some("stopped: cancelled")),
        "{outcome:?}"
    );
}

#[given("a cancelled instance")]
async fn cancelled_instance(world: &mut LibWorld) {
    world
        .start_fixture(
            "it",
            super::fixture("cancel.waiting"),
            json!(Waiting::Sleep),
        )
        .await;
    world.quiesce().await;
    world.handle(world.last()).cancel().await.expect("cancel");
    let last = world.last();
    world.outcome(last).await;
}

#[when("a new engine recovers the store")]
async fn new_engine_recovers(world: &mut LibWorld) {
    world.crash().await;
    world.recover().await;
}

#[then("the instance is not resumed")]
async fn not_resumed(world: &mut LibWorld) {
    let recovery = world.recovery.clone().expect("recovered");
    assert!(recovery.resumed.is_empty(), "{:?}", recovery.resumed);
    status_is(world, "Cancelled".into()).await;
}

// ---- compensation on cancel (error_handling.feature) ----

#[given("a workflow that registers compensations for each step it completes")]
fn registers_compensations(world: &mut LibWorld) {
    world
        .probe
        .script("reserve_hold", |s| s.block_on = Some(json!(3)));
    world.set_fixture(super::fixture("cancel.compensating"));
}

#[when(regex = r"^it is cancelled after (\w+) steps$")]
async fn cancelled_after(world: &mut LibWorld, n: String) {
    let n = number(&n);
    world.start("it", json!(null)).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("enough steps finished", || {
            probe.completed("reserve_hold") >= n
        })
        .await;
    world.handle(world.last()).cancel().await.expect("cancel");
}

#[then("the two compensations run in reverse order in a non-cancellable scope")]
async fn compensations_reverse(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await;
    let released: Vec<Value> = world
        .probe
        .calls_of("release_stock")
        .into_iter()
        .map(|c| c.input)
        .collect();
    assert_eq!(released, vec![json!("hold-2"), json!("hold-1")]);
}

/// "2" or "two".
pub fn number(text: &str) -> usize {
    text.parse().unwrap_or_else(|_| {
        [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        ]
        .iter()
        .position(|word| *word == text)
        .unwrap_or_else(|| panic!("not a number: {text}"))
    })
}
