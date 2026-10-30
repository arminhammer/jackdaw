//! tasks/child_workflows.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Receipt {
    pub charge: String,
}

pub struct Kids(pub Arc<Probe>);

#[tasks]
impl Kids {
    #[task(name = "kids.pay")]
    pub async fn pay(self: Arc<Self>, ctx: TaskContext, cents: u64) -> Result<String, TaskError> {
        self.0.typed("pay", &ctx, &cents).await
    }
    #[task(name = "kids.authorize")]
    pub async fn authorize(
        self: Arc<Self>,
        ctx: TaskContext,
        cents: u64,
    ) -> Result<String, TaskError> {
        self.0.typed("authorize", &ctx, &cents).await
    }
    #[task(name = "kids.ship")]
    pub async fn ship(
        self: Arc<Self>,
        ctx: TaskContext,
        parcel: String,
    ) -> Result<String, TaskError> {
        self.0.typed("ship_parcel", &ctx, &parcel).await
    }
    #[task(name = "kids.refund")]
    pub async fn refund(
        self: Arc<Self>,
        ctx: TaskContext,
        cents: u64,
    ) -> Result<String, TaskError> {
        self.0.typed("refund", &ctx, &cents).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("pay", |cents| json!(format!("ch-{cents}")));
    probe.default_output("authorize", |cents| json!(format!("auth-{cents}")));
    probe.default_output("ship_parcel", |parcel| {
        json!(format!("trk-{}", parcel.as_str().unwrap_or_default()))
    });
    probe.default_output("refund", |cents| json!(format!("rf-{cents}")));
}

#[workflow(namespace = "kids", name = "payment", version = "1")]
pub struct Payment;

#[workflow_methods]
impl Payment {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<Receipt> {
        ctx.call(Kids::authorize, &cents).await?;
        Ok(Receipt {
            charge: ctx.call(Kids::pay, &cents).await?,
        })
    }
}

#[workflow(namespace = "kids", name = "shipping", version = "1")]
pub struct Shipping;

#[workflow_methods]
impl Shipping {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, parcel: String) -> WorkflowResult<String> {
        Ok(ctx.call(Kids::ship, &parcel).await?)
    }
}

#[workflow(namespace = "kids", name = "order", version = "1")]
pub struct Order;

#[workflow_methods]
impl Order {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<Receipt> {
        let receipt = ctx.run(Payment::run, &cents).await?;
        Ok(receipt)
    }
}

#[workflow(namespace = "kids", name = "order-parallel", version = "1")]
pub struct OrderParallel;

#[workflow_methods]
impl OrderParallel {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<(Receipt, String)> {
        Ok(futures::try_join!(
            ctx.run(Payment::run, &cents),
            ctx.run(Shipping::run, &"parcel-1".to_string()),
        )?)
    }
}

#[workflow(namespace = "kids", name = "order-fan-out", version = "1")]
pub struct OrderFanOut;

#[workflow_methods]
impl OrderFanOut {
    #[run]
    async fn run(
        ctx: &mut WorkflowContext<Self>,
        parcels: Vec<String>,
    ) -> WorkflowResult<Vec<String>> {
        Ok(
            futures::future::try_join_all(parcels.iter().map(|p| ctx.run(Shipping::run, p)))
                .await?,
        )
    }
}

#[workflow(namespace = "kids", name = "order-handled", version = "1")]
pub struct OrderHandled;

#[workflow_methods]
impl OrderHandled {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, cents: u64) -> WorkflowResult<String> {
        match ctx.run(Payment::run, &cents).await {
            Ok(receipt) => Ok(receipt.charge),
            Err(StepError::Fault(_)) => Ok(ctx.call(Kids::refund, &cents).await?),
            Err(other) => Err(other),
        }
    }
}

#[workflow(namespace = "kids", name = "order-yaml", version = "1")]
pub struct OrderYaml;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Name {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Greeting {
    pub greeting: String,
}

pub const HELLO_YAML: &str = r#"
document:
  dsl: '1.0.3'
  namespace: kids
  name: hello
  version: '1.0.0'
do:
  - greet:
      set:
        greeting: '${ "Hello, " + .name + "!" }'
"#;

#[workflow_methods]
impl OrderYaml {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, name: String) -> WorkflowResult<String> {
        let workflow =
            Workflow::<Name, Greeting>::from_yaml(HELLO_YAML).map_err(StepError::Engine)?;
        let greeting = ctx.run(&workflow, &Name { name }).await?;
        Ok(greeting.greeting)
    }
}

#[workflow(namespace = "kids", name = "order-start", version = "1")]
pub struct OrderStart;

#[workflow_methods]
impl OrderStart {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, parcel: String) -> WorkflowResult<String> {
        Ok(ctx.start(Shipping::run, &parcel).await?)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Payment>("kids.payment").input(json!(500)),
        Fixture::of::<Shipping>("kids.shipping").input(json!("parcel-1")),
        Fixture::of::<Order>("kids.order").input(json!(500)),
        Fixture::of::<OrderParallel>("kids.order-parallel").input(json!(500)),
        Fixture::of::<OrderFanOut>("kids.order-fan-out"),
        Fixture::of::<OrderHandled>("kids.order-handled").input(json!(500)),
        Fixture::of::<OrderYaml>("kids.order-yaml").input(json!("Ada")),
        Fixture::of::<OrderStart>("kids.order-start").input(json!("parcel-9")),
    ]
}

#[given(regex = r#"^the workflows "Order", "Payment" and "Shipping" are registered$"#)]
fn registered(_world: &mut LibWorld) {}

#[given(regex = r#"^"Order" does "let receipt = ctx\.run\(Payment::run, &charge\)\.await\?"$"#)]
fn order_runs_payment(world: &mut LibWorld) {
    world.set_fixture(super::fixture("kids.order"));
}

#[when(regex = r#"^I execute "Order"$"#)]
async fn execute_order(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("order", input).await;
}

/// Instance ids of the child instances the probe saw, in order of first appearance.
fn child_instances(world: &LibWorld, task: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for call in world.probe.calls_of(task) {
        if !seen.contains(&call.instance) {
            seen.push(call.instance);
        }
    }
    seen
}

#[then(regex = r#"^"Payment" runs as its own instance$"#)]
async fn own_instance(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    let parent = world.runs[last].id.clone();
    let children = child_instances(world, "pay");
    assert_eq!(children.len(), 1);
    assert_ne!(children[0], parent);
    assert!(
        children[0].starts_with(&parent),
        "a child id is derived from its parent's"
    );
}

#[then(regex = r#"^"Order" receives the child's "Receipt"$"#)]
async fn receives_receipt(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({"charge": "ch-500"})
    );
}

#[given(
    regex = r#"^"Order" does "futures::try_join!\(ctx\.run\(Payment::run, &p\), ctx\.run\(Shipping::run, &s\)\)"$"#
)]
fn order_parallel(world: &mut LibWorld) {
    world
        .probe
        .script("pay", |s| s.delay = Some(Duration::from_millis(100)));
    world.probe.script("ship_parcel", |s| {
        s.delay = Some(Duration::from_millis(100))
    });
    world.set_fixture(super::fixture("kids.order-parallel"));
}

#[then("both children run at the same time")]
async fn both_at_once(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert!(world.probe.overlapped("pay", "ship_parcel"));
}

#[then(regex = r#"^"Order" receives both outputs$"#)]
async fn both_outputs(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!([{"charge": "ch-500"}, "trk-parcel-1"])
    );
}

#[given(regex = r#"^"Order" runs one "Shipping" child per parcel with "try_join_all"$"#)]
fn fan_out(world: &mut LibWorld) {
    world.set_fixture(super::fixture("kids.order-fan-out"));
}

#[when(regex = r"^I execute it with (\d+) parcels$")]
async fn with_parcels(world: &mut LibWorld, n: usize) {
    let parcels: Vec<String> = (1..=n).map(|i| format!("p{i}")).collect();
    world.start("order", json!(parcels)).await;
}

#[then(regex = r#"^(\d+) "Shipping" instances ran$"#)]
async fn shipping_instances(world: &mut LibWorld, n: usize) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(child_instances(world, "ship_parcel").len(), n);
}

#[given(regex = r#"^"Payment" fails with a "(\w+)" fault with status (\d+)$"#)]
fn payment_fails(world: &mut LibWorld, kind: String, status: u16) {
    world.probe.script("pay", |s| {
        s.fault = Some(FaultSpec::kind(&kind).status(status))
    });
}

#[when(regex = r#"^"Order" runs it with "\?"$"#)]
async fn order_with_question(world: &mut LibWorld) {
    world
        .start_fixture("order", super::fixture("kids.order"), json!(500))
        .await;
}

#[then(regex = r#"^"Order" fails with the same fault$"#)]
async fn same_fault(world: &mut LibWorld) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.kind, "payment");
    assert_eq!(fault.status, Some(402));
}

#[given(regex = r#"^"Payment" fails and "Order" matches the error and runs "Refund" instead$"#)]
fn handled(world: &mut LibWorld) {
    world.probe.script("pay", |s| {
        s.fault = Some(FaultSpec::kind("payment").status(402))
    });
    world.set_fixture(super::fixture("kids.order-handled"));
}

#[then(regex = r#"^"Order" completes through the "Refund" path$"#)]
async fn refund_path(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("rf-500"));
}

#[given(regex = r#"^"Order" runs "Payment" and only "Order" is registered$"#)]
fn only_order(world: &mut LibWorld) {
    world.only = Some(vec![super::fixture("kids.order").register]);
}

#[when("I build the engine")]
fn build_engine(world: &mut LibWorld) {
    world.build_error = world.builder().build().err().map(|e| e.to_string());
}

#[then(regex = r#"^the engine builds and runs "Payment" as a child of "Order"$"#)]
async fn builds_and_runs_child(world: &mut LibWorld) {
    assert_eq!(world.build_error, None);
    world
        .start_fixture("order", super::fixture("kids.order"), json!(500))
        .await;
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({"charge": "ch-500"})
    );
}

#[given(
    regex = r#"^"Order" does "ctx\.run\(&yaml_workflow, &input\)" with a typed "Workflow<In, Out>"$"#
)]
fn order_yaml(world: &mut LibWorld) {
    world.set_fixture(super::fixture("kids.order-yaml"));
}

#[then("the YAML workflow runs as a child and its output is typed")]
async fn yaml_typed(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("Hello, Ada!"));
}

#[given(regex = r#"^a completed "Order" instance$"#)]
async fn completed_order(world: &mut LibWorld) {
    world
        .start_fixture("order", super::fixture("kids.order"), json!(500))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
    world
        .notes
        .insert("children".into(), json!(child_instances(world, "pay")));
}

#[then("the replay refers to the same child instance ids")]
async fn same_child_ids(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
    let parent = world.runs[world.last()].id.clone();
    let children: Vec<String> = serde_json::from_value(world.notes["children"].clone()).unwrap();
    assert_eq!(children, vec![format!("{parent}/0")]);
}

#[given(regex = r#"^"Order" is waiting on a running "Payment" whose first step completed$"#)]
async fn order_waiting(world: &mut LibWorld) {
    world.probe.script("pay", |s| s.block = true);
    world
        .start_fixture("order", super::fixture("kids.order"), json!(500))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the payment's second step runs", || probe.running("pay"))
        .await;
}

#[when("the engine is dropped and a new engine recovers")]
async fn dropped_and_recovers(world: &mut LibWorld) {
    world.crash().await;
    world.probe.release_all();
    world.recover().await;
}

#[then(regex = r#"^"Payment" resumes without repeating its first step$"#)]
async fn payment_resumes(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.count("authorize"), 1);
    assert_eq!(world.probe.completed("pay"), 1);
}

#[then(regex = r#"^"Order" receives its output$"#)]
async fn order_receives(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({"charge": "ch-500"})
    );
}

#[then(regex = r#"^no second "Payment" instance is started$"#)]
fn no_second(world: &mut LibWorld) {
    assert_eq!(child_instances(world, "authorize").len(), 1);
    assert_eq!(child_instances(world, "pay").len(), 1);
    let recovery = world.recovery.as_ref().expect("recovered");
    assert_eq!(
        recovery.resumed.len(),
        1,
        "only the parent is resumed directly: {:?}",
        recovery.resumed
    );
}

#[given(regex = r#"^"Order" is waiting on a running "Shipping"$"#)]
async fn waiting_on_shipping(world: &mut LibWorld) {
    world.probe.script("ship_parcel", |s| s.block = true);
    world
        .start_fixture("order", super::fixture("kids.order-parallel"), json!(500))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("shipping runs", || probe.running("ship_parcel"))
        .await;
}

#[when(regex = r#"^"Order" is cancelled$"#)]
async fn order_cancelled(world: &mut LibWorld) {
    let handle = world.handle(world.last());
    handle.cancel().await.expect("cancel");
    let last = world.last();
    world.outcome(last).await;
}

#[then(regex = r#"^"Shipping" ends "Cancelled"$"#)]
async fn shipping_cancelled(world: &mut LibWorld) {
    let parent = world.runs[world.last()].id.clone();
    let engine = world.engine();
    let child = world
        .probe
        .calls_of("ship_parcel")
        .first()
        .map(|c| c.instance.clone())
        .expect("shipping ran");
    assert!(child.starts_with(&parent));
    let mut status = engine.status(&child).await.expect("status");
    for _ in 0..200 {
        if format!("{status:?}") == "Cancelled" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        status = engine.status(&child).await.expect("status");
    }
    assert_eq!(format!("{status:?}"), "Cancelled");
}

#[given(
    regex = r#"^"Order" starts "Shipping" with "ctx\.start\(Shipping::run, &s\)" and finishes$"#
)]
async fn order_starts(world: &mut LibWorld) {
    world.probe.script("ship_parcel", |s| s.block = true);
    world
        .start_fixture(
            "order",
            super::fixture("kids.order-start"),
            json!("parcel-9"),
        )
        .await;
}

#[then(regex = r#"^"Order" completes before "Shipping"$"#)]
async fn completes_before(world: &mut LibWorld) {
    let last = world.last();
    let id = world
        .outcome(last)
        .await
        .value()
        .as_str()
        .expect("an id")
        .to_string();
    world.notes.insert("started".into(), json!(id));
    let probe = Arc::clone(&world.probe);
    world
        .until_frozen("shipping runs", || probe.running("ship_parcel"))
        .await;
    assert_eq!(world.probe.completed("ship_parcel"), 0);
}

#[then(regex = r#"^"Shipping" keeps running with its own handle$"#)]
async fn keeps_running(world: &mut LibWorld) {
    let id = world.notes["started"].as_str().expect("an id").to_string();
    let engine = world.engine();
    let handle = super::fixture("kids.shipping").attach;
    let handle = handle(engine, id)
        .await
        .expect("a handle to the started workflow");
    world.probe.release("ship_parcel");
    assert_eq!(handle.await.expect("finishes"), json!("trk-parcel-9"));
    let _: Value = json!(null);
}
