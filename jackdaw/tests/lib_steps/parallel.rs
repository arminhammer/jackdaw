//! tasks/parallel_steps.feature and tasks/races.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Quote {
    pub price: u64,
    pub stock: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Approved {
    pub by: String,
}

pub struct Fan(pub Arc<Probe>);

#[tasks]
impl Fan {
    #[task(name = "par.price")]
    pub async fn price(self: Arc<Self>, ctx: TaskContext, sku: String) -> Result<u64, TaskError> {
        self.0.typed("price", &ctx, &sku).await
    }
    #[task(name = "par.stock")]
    pub async fn stock(self: Arc<Self>, ctx: TaskContext, sku: String) -> Result<u32, TaskError> {
        self.0.typed("stock", &ctx, &sku).await
    }
    #[task(name = "par.leaf")]
    pub async fn leaf(self: Arc<Self>, ctx: TaskContext, n: u32) -> Result<u32, TaskError> {
        self.0.typed("leaf", &ctx, &n).await
    }
    #[task(name = "par.primary")]
    pub async fn primary(
        self: Arc<Self>,
        ctx: TaskContext,
        q: String,
    ) -> Result<String, TaskError> {
        self.0.typed("primary", &ctx, &q).await
    }
    #[task(name = "par.fallback")]
    pub async fn fallback(
        self: Arc<Self>,
        ctx: TaskContext,
        q: String,
    ) -> Result<String, TaskError> {
        self.0.typed("fallback", &ctx, &q).await
    }
    #[task(name = "par.counted")]
    pub async fn counted(self: Arc<Self>, ctx: TaskContext, n: u32) -> Result<u32, TaskError> {
        self.0.typed("counted", &ctx, &n).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("stock", |_| json!(7));
    probe.default_output("leaf", |n| n.clone());
    probe.default_output("primary", |_| json!("primary"));
    probe.default_output("fallback", |_| json!("fallback"));
    probe.default_output("counted", |n| n.clone());
}

#[workflow(namespace = "par", name = "quote", version = "1")]
pub struct QuoteFlow;

#[workflow_methods]
impl QuoteFlow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, sku: String) -> WorkflowResult<Quote> {
        let (price, stock) =
            futures::try_join!(ctx.call(Fan::price, &sku), ctx.call(Fan::stock, &sku),)?;
        Ok(Quote { price, stock })
    }
}

#[workflow(namespace = "par", name = "join-all", version = "1")]
pub struct JoinAll;

#[workflow_methods]
impl JoinAll {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, skus: Vec<String>) -> WorkflowResult<Vec<u64>> {
        Ok(futures::future::try_join_all(skus.iter().map(|s| ctx.call(Fan::price, s))).await?)
    }
}

#[workflow(namespace = "par", name = "double", version = "1")]
pub struct Double;

#[workflow_methods]
impl Double {
    #[run]
    async fn run(_ctx: &mut WorkflowContext<Self>, n: u32) -> WorkflowResult<u32> {
        Ok(n * 2)
    }
}

#[workflow(namespace = "par", name = "mixed", version = "1")]
pub struct Mixed;

#[workflow_methods]
impl Mixed {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<(u32, u32, ())> {
        Ok(futures::try_join!(
            ctx.call(Fan::leaf, &1),
            ctx.run(Double::run, &21),
            ctx.sleep(1.secs()),
        )?)
    }
}

#[workflow(namespace = "par", name = "join-both", version = "1")]
pub struct JoinBoth;

#[workflow_methods]
impl JoinBoth {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, sku: String) -> WorkflowResult<Value> {
        let (price, stock) = futures::join!(ctx.call(Fan::price, &sku), ctx.call(Fan::stock, &sku));
        Ok(json!({ "stock": stock?, "price_error": price.err().map(|e| e.to_string()) }))
    }
}

#[workflow(namespace = "par", name = "nested", version = "1")]
pub struct Nested;

#[workflow_methods]
impl Nested {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        let left = async { futures::try_join!(ctx.call(Fan::leaf, &1), ctx.call(Fan::leaf, &2)) };
        let right = async { futures::try_join!(ctx.call(Fan::leaf, &3), ctx.call(Fan::leaf, &4)) };
        let ((a, b), (c, d)) = futures::try_join!(left, right)?;
        Ok(a + b + c + d)
    }
}

#[workflow(namespace = "par", name = "three", version = "1")]
pub struct Three;

#[workflow_methods]
impl Three {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        let (a, b, c) = futures::try_join!(
            ctx.call(Fan::counted, &1),
            ctx.call(Fan::counted, &2),
            ctx.call(Fan::counted, &3),
        )?;
        Ok(a + b + c)
    }
}

#[workflow(namespace = "par", name = "select", version = "1")]
pub struct SelectApproval;

#[workflow_methods]
impl SelectApproval {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(jackdaw::select! {
            approval = ctx.wait_for_event::<Approved>("approved") => format!("approved by {}", approval?.by),
            _ = ctx.sleep(24.hours()) => "expired".to_string(),
        })
    }
}

#[workflow(namespace = "par", name = "race", version = "1")]
pub struct RaceFallback;

#[workflow_methods]
impl RaceFallback {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, q: String) -> WorkflowResult<String> {
        Ok(
            match race(ctx.call(Fan::primary, &q), ctx.call(Fan::fallback, &q)).await {
                Either::Left(p) => p?,
                Either::Right(f) => f?,
            },
        )
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<QuoteFlow>("par.quote").input(json!("sku-1")),
        Fixture::of::<JoinAll>("par.join-all"),
        Fixture::of::<Double>("par.double").input(json!(21)),
        Fixture::of::<Mixed>("par.mixed"),
        Fixture::of::<JoinBoth>("par.join-both").input(json!("sku-1")),
        Fixture::of::<Nested>("par.nested"),
        Fixture::of::<Three>("par.three"),
        Fixture::of::<SelectApproval>("par.select"),
        Fixture::of::<RaceFallback>("par.race").input(json!("q")),
    ]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![
        ("let (price, stock) = futures::try_join!(", "par.quote"),
        (
            "approval = ctx.wait_for_event::<Approved>(\"approved\")",
            "par.select",
        ),
    ]
}

// ---- parallel steps ----

#[given(regex = r#"^tasks "Price" and "Stock" that each take (\d+) ms$"#)]
fn each_take(world: &mut LibWorld, ms: u64) {
    for name in ["price", "stock"] {
        world
            .probe
            .script(name, |s| s.delay = Some(Duration::from_millis(ms)));
    }
}

#[then(regex = r"^it completes in less than (\d+) ms$")]
async fn completes_within(world: &mut LibWorld, ms: u64) {
    let last = world.last();
    world.outcome(last).await.value();
    let took = world.clock.elapsed();
    assert!(
        took < Duration::from_millis(ms),
        "took {took:?} of virtual time"
    );
}

#[then("the quote has both results")]
async fn quote_both(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({"price": 500, "stock": 7})
    );
}

#[given(regex = r#"^a workflow that does "futures::future::try_join_all\(.+\)\.await\?"$"#)]
fn join_all(world: &mut LibWorld) {
    world
        .probe
        .script("price", |s| s.delay = Some(Duration::from_millis(100)));
    world.set_fixture(super::fixture("par.join-all"));
}

fn skus(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("sku-{}", "x".repeat(i))).collect()
}

#[when(regex = r"^I execute it with (\d+) skus$")]
async fn execute_skus(world: &mut LibWorld, n: usize) {
    world.start("it", json!(skus(n))).await;
}

#[then(regex = r"^(\d+) prices come back in sku order$")]
async fn prices_in_sku_order(world: &mut LibWorld, n: usize) {
    let last = world.last();
    let expected: Vec<u64> = skus(n).iter().map(|s| s.len() as u64 * 100).collect();
    assert_eq!(world.outcome(last).await.value(), &json!(expected));
}

#[then(regex = r"^the (\d+) calls overlapped in time$")]
async fn calls_overlapped(world: &mut LibWorld, n: usize) {
    world.settle().await;
    assert_eq!(world.probe.max_in_flight("price"), n);
}

#[given("a workflow that try_joins a task call, a child workflow and a 1-second sleep")]
fn mixed(world: &mut LibWorld) {
    world.set_fixture(super::fixture("par.mixed"));
}

#[then("all three complete and the results are returned as a tuple")]
async fn tuple(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!([1, 42, null]));
}

#[given(
    regex = r#"^a try_join of "Price", which fails non-retryably, and "Stock", which takes 1 second$"#
)]
fn price_fails(world: &mut LibWorld) {
    world.probe.script("price", |s| {
        s.fault = Some(FaultSpec::non_retryable("price service down"))
    });
    world
        .probe
        .script("stock", |s| s.delay = Some(Duration::from_secs(1)));
    world.set_fixture(super::fixture("par.quote"));
}

#[then(regex = r#"^the workflow fails with "Price"'s fault$"#)]
async fn fails_with_price(world: &mut LibWorld) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.detail.as_deref(), Some("price service down"));
}

#[given(regex = r#"^a "futures::join!" of "Price", which fails, and "Stock", which succeeds$"#)]
fn join_both(world: &mut LibWorld) {
    world.probe.script("price", |s| {
        s.fault = Some(FaultSpec::non_retryable("price service down"))
    });
    world.set_fixture(super::fixture("par.join-both"));
}

#[when("the workflow inspects both results")]
async fn inspects(world: &mut LibWorld) {
    world.start("it", json!("sku-1")).await;
}

#[then(regex = r#"^it can continue with "Stock"'s value and report "Price"'s error$"#)]
async fn continue_with_stock(world: &mut LibWorld) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    assert_eq!(value["stock"], json!(7));
    assert!(
        value["price_error"]
            .as_str()
            .is_some_and(|e| e.contains("price service down")),
        "{value}"
    );
}

#[given(
    regex = r#"^a completed instance whose two parallel steps finished in the order "Stock", "Price"$"#
)]
async fn completed_in_order(world: &mut LibWorld) {
    world
        .probe
        .script("price", |s| s.delay = Some(Duration::from_millis(200)));
    world
        .probe
        .script("stock", |s| s.delay = Some(Duration::from_millis(100)));
    world
        .start_fixture("it", super::fixture("par.quote"), json!("sku-1"))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[when("I replay it with the steps now finishing in the opposite order")]
async fn replay_opposite(world: &mut LibWorld) {
    world
        .probe
        .script("price", |s| s.delay = Some(Duration::from_millis(100)));
    world
        .probe
        .script("stock", |s| s.delay = Some(Duration::from_millis(200)));
    super::common::replay_all(world).await;
}

#[given(regex = r"^a try_join of three counting tasks where the first two have finished$")]
async fn three_two_done(world: &mut LibWorld) {
    world
        .probe
        .script("counted", |s| s.block_on = Some(json!(3)));
    world
        .start_fixture("it", super::fixture("par.three"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("two tasks finished", || probe.completed("counted") >= 2)
        .await;
}

#[when("the engine is dropped and a new engine resumes the instance")]
async fn dropped_and_resumed(world: &mut LibWorld) {
    world.crash().await;
    world.probe.release("counted");
    let last = world.last();
    world.resume(last).await;
}

#[then("the first two tasks ran once in total")]
async fn first_two_once(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    for n in [1, 2] {
        let runs = world
            .probe
            .calls_of("counted")
            .iter()
            .filter(|c| c.input == json!(n))
            .count();
        assert_eq!(runs, 1, "task {n}");
    }
}

#[then("the third task runs to completion")]
async fn third_completes(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(6));
}

#[given("a workflow that try_joins two blocks that each try_join two task calls")]
fn nested(world: &mut LibWorld) {
    world
        .probe
        .script("leaf", |s| s.delay = Some(Duration::from_millis(100)));
    world.set_fixture(super::fixture("par.nested"));
}

#[then("all four calls overlapped in time")]
async fn four_overlapped(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(10));
    assert_eq!(world.probe.max_in_flight("leaf"), 4);
}

// ---- races ----

#[when(regex = r#"^the event "approved" from "(\w+)" is sent after (.+)$"#)]
async fn event_after(world: &mut LibWorld, by: String, after: String) {
    world.start("it", json!(null)).await;
    world.pass(duration(&after)).await;
    let last = world.last();
    world
        .handle(last)
        .send_event("approved", Approved { by })
        .await
        .expect("send_event");
}

#[given("the same workflow")]
fn same_workflow(world: &mut LibWorld) {
    world.set_fixture(super::fixture("par.select"));
}

#[when(regex = r"^(\d+ \w+) pass without the event$")]
async fn pass_without(world: &mut LibWorld, span: String) {
    if world.runs.is_empty() {
        world.start("it", json!(null)).await;
    }
    world.pass(duration(&span)).await;
}

#[given("a completed instance where the event won the race")]
async fn event_won(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("par.select"), json!(null))
        .await;
    let last = world.last();
    world
        .handle(last)
        .send_event("approved", Approved { by: "ada".into() })
        .await
        .expect("send");
    assert_eq!(world.outcome(last).await.value(), &json!("approved by ada"));
}

#[then("the replay takes the event branch")]
fn replay_event_branch(world: &mut LibWorld) {
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
}

#[then("no timer is started during the replay")]
fn no_timer(world: &mut LibWorld) {
    let before = world
        .notes
        .get("sleeps")
        .and_then(Value::as_u64)
        .expect("counted before replay");
    assert_eq!(world.clock.sleeps_started(), before);
}

#[given(
    regex = r#"^a workflow that does "match race\(ctx\.call\(Primary, &q\), ctx\.call\(Fallback, &q\)\)\.await .+"$"#
)]
fn race_fallback(world: &mut LibWorld) {
    world.set_fixture(super::fixture("par.race"));
}

#[given(regex = r#"^"Primary" takes 2 seconds and "Fallback" takes 100 ms$"#)]
fn primary_slow(world: &mut LibWorld) {
    world
        .probe
        .script("primary", |s| s.delay = Some(Duration::from_secs(2)));
    world
        .probe
        .script("fallback", |s| s.delay = Some(Duration::from_millis(100)));
}

#[then(regex = r#"^the workflow uses the "Fallback" result$"#)]
async fn uses_fallback(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("fallback"));
}

#[given("a race between a 100 ms task and a task that waits for cancellation")]
fn race_cancel(world: &mut LibWorld) {
    world.probe.script("primary", |s| s.block = true);
    world
        .probe
        .script("fallback", |s| s.delay = Some(Duration::from_millis(100)));
    world.set_fixture(super::fixture("par.race"));
}

#[when("the 100 ms task wins")]
async fn hundred_wins(world: &mut LibWorld) {
    world.start("it", json!("q")).await;
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("fallback"));
}

#[then("the losing task observed cancellation and did not complete")]
async fn loser_cancelled(world: &mut LibWorld) {
    let probe = Arc::clone(&world.probe);
    world
        .until("the loser is cancelled", || {
            probe.calls_of("primary").iter().all(|c| c.cancelled)
        })
        .await;
    assert_eq!(world.probe.count("primary"), 1);
}

#[given(regex = r"^the select! of an event and a 24-hour timer has waited (\d+) hours$")]
async fn waited_hours(world: &mut LibWorld, hours: u64) {
    world
        .start_fixture("it", super::fixture("par.select"), json!(null))
        .await;
    world.pass(Duration::from_secs(hours * 3600)).await;
}

#[when(regex = r"^(\d+) more hours pass without the event$")]
async fn more_hours(world: &mut LibWorld, hours: u64) {
    world.pass(Duration::from_secs(hours * 3600)).await;
}

#[given("the select! of an event and a 24-hour timer")]
fn select_timer(world: &mut LibWorld) {
    world.set_fixture(super::fixture("par.select"));
}

#[when("the event is sent and the engine is dropped before the race resolves")]
async fn sent_then_dropped(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    let last = world.last();
    let handle = world.handle(last);
    // Stop the instance from consuming the event before the "crash": drop the engine right after the event is stored.
    handle
        .send_event("approved", Approved { by: "ada".into() })
        .await
        .expect("send");
    drop(handle);
    world.crash().await;
}

#[then("the event branch wins")]
async fn event_branch_wins(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("approved by ada"));
}
