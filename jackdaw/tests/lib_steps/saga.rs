//! workflows/order_saga.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Order {
    pub id: String,
    pub items: Vec<String>,
    pub cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    pub kind: String,
    pub order: String,
}

impl Message {
    fn declined(order: &Order) -> Self {
        Self {
            kind: "declined".into(),
            order: order.id.clone(),
        }
    }
    fn shipped(order: &Order, shipment: &str) -> Self {
        Self {
            kind: format!("shipped {shipment}"),
            order: order.id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Receipt {
    pub charge: String,
    pub shipment: String,
}

pub struct Saga(pub Arc<Probe>);

// Probe keys are shared with errors.rs (reserve_stock, release_stock, notify), calling_tasks.rs (charge) and
// children.rs (refund); their defaults fit these signatures.
#[tasks]
impl Saga {
    #[task(name = "saga.reserve_stock")]
    pub async fn reserve_stock(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Order,
    ) -> Result<String, TaskError> {
        self.0.typed("reserve_stock", &ctx, &order).await
    }
    #[task(name = "saga.release_stock")]
    pub async fn release_stock(
        self: Arc<Self>,
        ctx: TaskContext,
        hold: String,
    ) -> Result<(), TaskError> {
        self.0.typed("release_stock", &ctx, &hold).await
    }
    #[task(name = "saga.charge")]
    pub async fn charge(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Order,
    ) -> Result<String, TaskError> {
        self.0.typed("charge", &ctx, &order).await
    }
    #[task(name = "saga.refund")]
    pub async fn refund(
        self: Arc<Self>,
        ctx: TaskContext,
        charge: String,
    ) -> Result<String, TaskError> {
        self.0.typed("refund", &ctx, &charge).await
    }
    #[task(name = "saga.create_shipment")]
    pub async fn create_shipment(
        self: Arc<Self>,
        ctx: TaskContext,
        order: (Order, String),
    ) -> Result<String, TaskError> {
        self.0.typed("create_shipment", &ctx, &order).await
    }
    #[task(name = "saga.cancel_shipment")]
    pub async fn cancel_shipment(
        self: Arc<Self>,
        ctx: TaskContext,
        shipment: String,
    ) -> Result<(), TaskError> {
        self.0.typed("cancel_shipment", &ctx, &shipment).await
    }
    #[task(name = "saga.notify")]
    pub async fn notify(
        self: Arc<Self>,
        ctx: TaskContext,
        message: Message,
    ) -> Result<(), TaskError> {
        self.0.typed("notify", &ctx, &message).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("create_shipment", |_| json!("shp-1"));
    probe.default_output("cancel_shipment", |_| json!(null));
}

#[workflow(namespace = "shop", name = "order-saga", version = "1.0.0")]
pub struct OrderSaga;

#[workflow_methods]
impl OrderSaga {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Receipt> {
        let hold = ctx
            .call(Saga::reserve_stock, &order)
            .retry(Retry::exponential(5))
            .await?;

        let charge = match ctx
            .call(Saga::charge, &order)
            .retry(Retry::exponential(3))
            .timeout(30.secs())
            .await
        {
            Ok(charge) => charge,
            Err(error) => {
                ctx.call(Saga::release_stock, &hold).await?;
                ctx.call(Saga::notify, &Message::declined(&order)).await?;
                return Err(error);
            }
        };

        let shipment = match ctx
            .call(Saga::create_shipment, &(order.clone(), hold.clone()))
            .await
        {
            Ok(shipment) => shipment,
            Err(error) => {
                futures::try_join!(
                    ctx.call(Saga::refund, &charge),
                    ctx.call(Saga::release_stock, &hold)
                )?;
                return Err(error);
            }
        };

        ctx.call(Saga::notify, &Message::shipped(&order, &shipment))
            .await?;
        Ok(Receipt { charge, shipment })
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![Fixture::of::<OrderSaga>("saga.order").input(order_json("o-1", 2))]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![("struct OrderSaga", "saga.order")]
}

fn order_json(id: &str, items: usize) -> Value {
    json!({ "id": id, "items": (0..items).map(|i| format!("item-{i}")).collect::<Vec<_>>(), "cents": 500 })
}

fn names(list: &str) -> Vec<String> {
    list.split(',')
        .map(|name| name.trim().to_string())
        .collect()
}

#[given(regex = r#"^the tasks "ReserveStock", .+ that count their invocations$"#)]
fn saga_tasks(_world: &mut LibWorld) {}

#[when(regex = r#"^I execute "OrderSaga"(?: for an order of (\d+) items)?$"#)]
async fn execute_saga(world: &mut LibWorld, items: String) {
    let items = items.parse().unwrap_or(2);
    // The compensations take a moment, so running them in parallel is observable.
    for task in ["refund", "release_stock"] {
        world.probe.script(task, |s| {
            s.delay = s.delay.or(Some(Duration::from_millis(10)))
        });
    }
    world
        .start_fixture("it", super::fixture("saga.order"), order_json("o-1", items))
        .await;
}

#[then("the workflow completes with a receipt")]
async fn completes_with_receipt(world: &mut LibWorld) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    assert!(
        value["charge"].as_str().is_some() && value["shipment"] == json!("shp-1"),
        "{value}"
    );
}

#[then(regex = r"^these tasks ran exactly once: (.+)$")]
async fn ran_exactly_once(world: &mut LibWorld, list: String) {
    world.settle().await;
    for name in names(&list) {
        assert_eq!(world.probe.count(&name), 1, "{name}");
    }
}

#[then(regex = r"^these tasks never ran: (.+)$")]
async fn never_ran_list(world: &mut LibWorld, list: String) {
    world.settle().await;
    for name in names(&list) {
        assert_eq!(world.probe.count(&name), 0, "{name}");
    }
}

#[given(regex = r#"^"(\w+)" fails retryably twice$"#)]
fn fails_retryably_twice(world: &mut LibWorld, task: String) {
    world.probe.script(&task, |s| s.fail_first = 2);
}

#[given(regex = r#"^"Charge" fails with "TaskError::fault\(\\"(\w+)\\"\)\.status\((\d+)\)"$"#)]
fn charge_fault(world: &mut LibWorld, kind: String, status: u16) {
    world.probe.script("charge", |s| {
        s.fault = Some(FaultSpec::kind(&kind).status(status))
    });
}

#[then(regex = r"^the workflow fails with the (\d+) fault$")]
async fn fails_with_status(world: &mut LibWorld, status: u16) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.fault().status, Some(status));
}

#[then(regex = r#"^"ReleaseStock" ran once with the hold$"#)]
async fn released_hold(world: &mut LibWorld) {
    world.settle().await;
    let calls = world.probe.calls_of("release_stock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input, json!("hold-1"));
}

#[then(regex = r#"^"Notify" ran once with a "(\w+)" message$"#)]
async fn notified(world: &mut LibWorld, kind: String) {
    world.settle().await;
    let calls = world.probe.calls_of("notify");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["kind"], json!(kind));
}

#[given(regex = r#"^"(\w+)" fails non-retryably$"#)]
fn fails_non_retryably(world: &mut LibWorld, task: String) {
    let detail = format!("{task} failed");
    world
        .probe
        .script(&task, |s| s.fault = Some(FaultSpec::non_retryable(&detail)));
}

#[then(regex = r#"^"Refund" and "ReleaseStock" each ran once and overlapped in time$"#)]
async fn refund_release_overlap(world: &mut LibWorld) {
    world.settle().await;
    assert_eq!(world.probe.count("refund"), 1);
    assert_eq!(world.probe.count("release_stock"), 1);
    assert!(world.probe.overlapped("refund", "release_stock"));
}

#[then("the workflow fails with the shipment fault")]
async fn shipment_fault(world: &mut LibWorld) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.detail.as_deref(), Some("CreateShipment failed"));
}

#[given(regex = r#"^"(\w+)" takes (\d+ \w+)$"#)]
fn takes(world: &mut LibWorld, task: String, span: String) {
    let delay = duration(&span);
    world.probe.script(&task, |s| s.delay = Some(delay));
}

#[then("the workflow fails with a timeout")]
async fn fails_timeout(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert!(matches!(outcome, Outcome::Timeout(_)), "{outcome:?}");
}

#[given(regex = r#"^"(\w+)" blocks until released$"#)]
fn blocks_until_released(world: &mut LibWorld, task: String) {
    world.probe.script(&task, |s| s.block = true);
}

#[when(regex = r#"^the engine is dropped while "(\w+)" runs$"#)]
async fn dropped_while(world: &mut LibWorld, task: String) {
    if world.runs.is_empty() {
        world
            .start_fixture("it", super::fixture("saga.order"), order_json("o-1", 2))
            .await;
    }
    let probe = Arc::clone(&world.probe);
    world.until("the task runs", || probe.running(&task)).await;
    world.crash().await;
}

#[when(regex = r#"^a new engine resumes the instance and "(\w+)" is released$"#)]
async fn resume_and_release(world: &mut LibWorld, task: String) {
    world.probe.release(&task);
    let last = world.last();
    world.resume(last).await;
}

#[then(regex = r#"^"(\w+)" ran exactly once in total$"#)]
async fn exactly_once_total(world: &mut LibWorld, task: String) {
    world.settle().await;
    assert_eq!(world.probe.count(&task), 1);
}

#[then(regex = r#"^"(\w+)" received the same idempotency key in every attempt$"#)]
fn same_key_every_attempt(world: &mut LibWorld, task: String) {
    let keys: std::collections::HashSet<String> = world
        .probe
        .calls_of(&task)
        .into_iter()
        .map(|c| c.key)
        .collect();
    assert_eq!(keys.len(), 1);
}

#[given(regex = r#"^"CreateShipment" fails and "Refund" blocks until released$"#)]
fn ship_fails_refund_blocks(world: &mut LibWorld) {
    world.probe.script("create_shipment", |s| {
        s.fault = Some(FaultSpec::non_retryable("CreateShipment failed"))
    });
    world.probe.script("refund", |s| s.block = true);
}

#[then(regex = r#"^"Refund" and "ReleaseStock" each completed once in total$"#)]
async fn each_completed_once(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.fault();
    assert_eq!(world.probe.completed("refund"), 1);
    assert_eq!(world.probe.completed("release_stock"), 1);
}

#[when(
    regex = r#"^I execute (\d+) "OrderSaga" instances at once, where every 5th card is declined$"#
)]
async fn fifty_orders(world: &mut LibWorld, n: usize) {
    world.probe.script("charge", |s| {
        for i in (4..n).step_by(5) {
            s.fail_on.push((
                order_json(&format!("o-{i}"), 2),
                FaultSpec::kind("payment").status(402),
                None,
            ));
        }
    });
    for i in 0..n {
        world
            .start_fixture(
                &format!("o{i}"),
                super::fixture("saga.order"),
                order_json(&format!("o-{i}"), 2),
            )
            .await;
    }
}

#[then(regex = r"^(\d+) complete and (\d+) fail$")]
async fn complete_and_fail(world: &mut LibWorld, ok: usize, failed: usize) {
    world.settle().await;
    let done = world
        .runs
        .iter()
        .filter(|r| matches!(r.outcome, Some(Outcome::Ok(_))))
        .count();
    assert_eq!((done, world.runs.len() - done), (ok, failed));
}

#[then(regex = r#"^"(\w+)" ran exactly (\d+) times$"#)]
async fn ran_exactly_n(world: &mut LibWorld, task: String, n: usize) {
    world.settle().await;
    assert_eq!(world.probe.count(&task), n);
}

#[then("each completed instance's history shows its own tasks only")]
async fn own_history(world: &mut LibWorld) {
    let engine = world.engine();
    for run in &world.runs {
        if !matches!(run.outcome, Some(Outcome::Ok(_))) {
            continue;
        }
        let history = engine.history(&run.id).await.expect("history");
        assert!(history.iter().all(|event| event.instance_id() == run.id));
        let completed = history
            .iter()
            .filter(|e| matches!(e, Event::TaskCompleted { .. }))
            .count();
        assert_eq!(completed, 4, "reserve, charge, ship and notify");
    }
}

#[given(regex = r#"^(\d+) completed or failed "OrderSaga" instances$"#)]
async fn twenty_instances(world: &mut LibWorld, n: usize) {
    world.probe.script("charge", |s| {
        for i in (0..n).step_by(3) {
            s.fail_on.push((
                order_json(&format!("o-{i}"), 2),
                FaultSpec::kind("payment").status(402),
                None,
            ));
        }
    });
    for i in 0..n {
        world
            .start_fixture(
                &format!("o{i}"),
                super::fixture("saga.order"),
                order_json(&format!("o-{i}"), 2),
            )
            .await;
    }
    world.settle().await;
    let _ = Duration::ZERO;
}
