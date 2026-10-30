//! tasks/error_handling.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ChargeOutcome {
    Charged(String),
    Declined,
}

pub struct Errs(pub Arc<Probe>);

#[tasks]
impl Errs {
    #[task(name = "err.charge")]
    pub async fn charge(
        self: Arc<Self>,
        ctx: TaskContext,
        cents: u64,
    ) -> Result<String, TaskError> {
        self.0.typed("charge", &ctx, &cents).await
    }
    #[task(name = "err.notify")]
    pub async fn notify(self: Arc<Self>, ctx: TaskContext, text: String) -> Result<(), TaskError> {
        self.0.typed("notify", &ctx, &text).await
    }
    #[task(name = "err.slow")]
    pub async fn slow(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("slow", &ctx, &()).await
    }
    #[task(name = "err.boom")]
    pub async fn boom(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("boom", &ctx, &()).await
    }
    #[task(name = "err.reserve_stock")]
    pub async fn reserve_stock(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Value,
    ) -> Result<String, TaskError> {
        self.0.typed("reserve_stock", &ctx, &order).await
    }
    #[task(name = "err.release_stock")]
    pub async fn release_stock(
        self: Arc<Self>,
        ctx: TaskContext,
        hold: String,
    ) -> Result<(), TaskError> {
        self.0.typed("release_stock", &ctx, &hold).await
    }
    #[task(name = "err.charge_order")]
    pub async fn charge_order(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Value,
    ) -> Result<String, TaskError> {
        self.0.typed("charge", &ctx, &order).await
    }
}

pub fn defaults(probe: &Probe) {
    for name in ["notify", "slow", "boom", "release_stock"] {
        probe.default_output(name, |_| json!(null));
    }
    probe.default_output("reserve_stock", |_| json!("hold-1"));
}

#[workflow(namespace = "err", name = "propagate", version = "1")]
pub struct Propagate;

#[workflow_methods]
impl Propagate {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
        Ok(ctx.call(Errs::charge, &cents).await?)
    }
}

#[workflow(namespace = "err", name = "handle", version = "1")]
pub struct HandleDecline;

#[workflow_methods]
impl HandleDecline {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<ChargeOutcome> {
        match ctx
            .call(Errs::charge, &cents)
            .retry(Retry::exponential(5))
            .await
        {
            Ok(id) => Ok(ChargeOutcome::Charged(id)),
            Err(StepError::Fault(fault)) if fault.status == Some(402) => {
                ctx.call(Errs::notify, &"card declined".to_string()).await?;
                Ok(ChargeOutcome::Declined)
            }
            Err(other) => Err(other),
        }
    }
}

#[workflow(namespace = "err", name = "retrying", version = "1")]
pub struct Retrying;

#[workflow_methods]
impl Retrying {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
        Ok(ctx
            .call(Errs::charge, &cents)
            .retry(Retry::exponential(5))
            .await?)
    }
}

#[workflow(namespace = "err", name = "timeout", version = "1")]
pub struct TimeoutMatch;

#[workflow_methods]
impl TimeoutMatch {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(match ctx.call(Errs::slow, &()).timeout(1.secs()).await {
            Ok(()) => "ok".into(),
            Err(StepError::Timeout { .. }) => "Err(StepError::Timeout { .. })".into(),
            Err(other) => format!("other: {other}"),
        })
    }
}

#[workflow(namespace = "err", name = "panic", version = "1")]
pub struct Panicking;

#[workflow_methods]
impl Panicking {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        Ok(ctx.call(Errs::boom, &()).await?)
    }
}

#[workflow(namespace = "err", name = "own-fault", version = "1")]
pub struct OwnFault;

#[workflow_methods]
impl OwnFault {
    #[run]
    async fn run(_ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        Err(TaskError::fault("validation")
            .status(400)
            .title("Empty order")
            .into())
    }
}

#[workflow(namespace = "err", name = "compensate", version = "1")]
pub struct Compensate;

#[workflow_methods]
impl Compensate {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: Value) -> WorkflowResult<String> {
        let hold = ctx.call(Errs::reserve_stock, &order).await?;
        let charge = match ctx.call(Errs::charge_order, &order).await {
            Ok(c) => c,
            Err(e) => {
                ctx.call(Errs::release_stock, &hold).await?;
                return Err(e);
            }
        };
        Ok(charge)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Propagate>("err.propagate").input(json!(500)),
        Fixture::of::<HandleDecline>("err.handle").input(json!(500)),
        Fixture::of::<Retrying>("err.retrying").input(json!(500)),
        Fixture::of::<TimeoutMatch>("err.timeout"),
        Fixture::of::<Panicking>("err.panic"),
        Fixture::of::<OwnFault>("err.own-fault"),
        Fixture::of::<Compensate>("err.compensate").input(json!({"id": "o-1"})),
    ]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Err(StepError::Fault(fault)) if fault.status == Some(402)",
            "err.handle",
        ),
        ("let hold = ctx.call(ReserveStock", "err.compensate"),
    ]
}

#[given(
    regex = r#"^a workflow that does "ctx\.call\(Charge, &cents\)\.await\?" and "Charge" fails with "TaskError::non_retryable\(\\"(.+)\\"\)"$"#
)]
fn propagate(world: &mut LibWorld, detail: String) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::non_retryable(&detail))
    });
    world.set_fixture(super::fixture("err.propagate"));
}

#[then(regex = r#"^awaiting the handle fails with "Error::Faulted" whose detail contains "(.+)"$"#)]
async fn faulted_with_detail(world: &mut LibWorld, text: String) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert!(
        fault.detail.as_deref().unwrap_or_default().contains(&text),
        "{fault:?}"
    );
}

#[when(regex = r#"^"Charge" fails with "TaskError::fault\(\\"(\w+)\\"\)\.status\((\d+)\)"$"#)]
async fn charge_fault_status(world: &mut LibWorld, kind: String, status: u16) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::kind(&kind).status(status))
    });
    let input = world.fixture().input;
    world.start("it", input).await;
}

#[given(regex = r#"^"Charge" fails with "TaskError::retryable" on attempts 1 and 2$"#)]
fn retryable_twice(world: &mut LibWorld) {
    world.probe.script("charge", |s| s.fail_first = 2);
}

#[given(regex = r#"^"Charge" fails with "TaskError::non_retryable"$"#)]
fn non_retryable(world: &mut LibWorld) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::non_retryable("declined"))
    });
}

#[when(regex = r#"^a workflow calls it with "\.retry\(Retry::exponential\(5\)\)"$"#)]
async fn calls_with_retry(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("err.retrying"), json!(500))
        .await;
}

#[then(regex = r"^the step succeeds on attempt (\d+)$")]
async fn succeeds_on(world: &mut LibWorld, attempt: u32) {
    let last = world.last();
    world.outcome(last).await.value();
    let attempts: Vec<u32> = world
        .probe
        .calls_of("charge")
        .iter()
        .map(|c| c.attempt)
        .collect();
    assert_eq!(attempts.last(), Some(&attempt), "{attempts:?}");
}

#[then(regex = r#"^the events include (\d+) "TaskRetried" events$"#)]
async fn retried_events(world: &mut LibWorld, n: usize) {
    let last = world.last();
    world.outcome(last).await;
    let mut events = world.handle(last).events();
    let mut retried = 0;
    while let Some(event) = events.next().await {
        if matches!(event, Event::TaskRetried { .. }) {
            retried += 1;
        }
    }
    assert_eq!(retried, n);
}

#[then(regex = r"^the step fails after (\d+) attempts?$")]
async fn fails_after(world: &mut LibWorld, n: usize) {
    let last = world.last();
    world.outcome(last).await.fault();
    assert_eq!(world.probe.count("charge"), n);
}

#[given("a task that takes 10 seconds")]
fn takes_ten(world: &mut LibWorld) {
    world
        .probe
        .script("slow", |s| s.delay = Some(Duration::from_secs(10)));
}

#[when(regex = r#"^a workflow calls it with "\.timeout\(1\.secs\(\)\)" and matches the result$"#)]
async fn calls_with_timeout(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("err.timeout"), json!(null))
        .await;
}

#[then(regex = r#"^the result is "(.+)"$"#)]
async fn result_is(world: &mut LibWorld, expected: String) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(expected));
}

#[given(regex = r#"^a task that panics with "(.+)"$"#)]
fn panics_with(world: &mut LibWorld, message: String) {
    world.probe.script("boom", |s| s.panic = Some(message));
}

#[when("a workflow calls it")]
async fn a_workflow_calls_it(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("err.panic"), json!(null))
        .await;
}

#[then(regex = r#"^the step fails with a runtime fault whose detail contains "(.+)"$"#)]
async fn runtime_fault(world: &mut LibWorld, text: String) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.kind, error_types::RUNTIME);
    assert!(
        fault.detail.as_deref().unwrap_or_default().contains(&text),
        "{fault:?}"
    );
}

#[then("other running instances are unaffected")]
async fn others_unaffected(world: &mut LibWorld) {
    world
        .start_fixture("other", super::fixture("calls.repeat"), json!(null))
        .await;
    let other = world.run_index("other");
    world.outcome(other).await.value();
}

#[given(regex = r#"^a workflow that returns "Err\(TaskError::fault.+"$"#)]
fn own_fault(world: &mut LibWorld) {
    world.set_fixture(super::fixture("err.own-fault"));
}

#[then(
    regex = r#"^awaiting the handle fails with a validation fault with status (\d+) and title "(.+)"$"#
)]
async fn validation_fault(world: &mut LibWorld, status: u16, title: String) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert!(fault.kind.contains("validation"), "{fault:?}");
    assert_eq!(fault.status, Some(status));
    assert_eq!(fault.title.as_deref(), Some(title.as_str()));
}

#[when(regex = r#"^"Charge" fails non-retryably$"#)]
async fn charge_fails(world: &mut LibWorld) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::non_retryable("card declined"))
    });
    let input = world.fixture().input;
    world.start("it", input).await;
}

#[then(regex = r#"^"release_stock" ran once with the hold from "reserve_stock"$"#)]
async fn released_hold(world: &mut LibWorld) {
    world.settle().await;
    let releases = world.probe.calls_of("release_stock");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].input, json!("hold-1"));
}

#[given(
    regex = r#"^the compensation step "ReleaseStock" was running when the engine was dropped$"#
)]
async fn compensation_interrupted(world: &mut LibWorld) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::non_retryable("card declined"))
    });
    world.probe.script("release_stock", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("err.compensate"), json!({"id": "o-1"}))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the compensation runs", || probe.running("release_stock"))
        .await;
    world.crash().await;
    world.probe.release("release_stock");
}

#[then(regex = r#"^"release_stock" completes$"#)]
async fn release_completes(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.fault();
    assert_eq!(world.probe.completed("release_stock"), 1);
}

#[then(regex = r#"^"reserve_stock" and "charge" are not run again$"#)]
fn not_again(world: &mut LibWorld) {
    assert_eq!(world.probe.count("reserve_stock"), 1);
    assert_eq!(world.probe.count("charge"), 1);
}

#[given(regex = r"^a completed instance whose step succeeded on attempt 3$")]
async fn succeeded_on_three(world: &mut LibWorld) {
    world.probe.script("charge", |s| s.fail_first = 2);
    world
        .start_fixture("it", super::fixture("err.retrying"), json!(500))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[then("the task does not run")]
fn task_does_not_run(world: &mut LibWorld) {
    assert_eq!(Some(world.probe.calls().len()), world.mark);
}
