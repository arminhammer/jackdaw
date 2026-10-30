//! tasks/calling_tasks.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The feature's tasks: `reserve` and `charge` count their calls through the probe.
pub struct Calls(pub Arc<Probe>);

#[tasks]
impl Calls {
    #[task]
    pub async fn reserve(self: Arc<Self>, ctx: TaskContext, sku: String) -> Result<u32, TaskError> {
        self.0.typed("reserve", &ctx, &sku).await
    }

    #[task]
    pub async fn charge(
        self: Arc<Self>,
        ctx: TaskContext,
        cents: u64,
    ) -> Result<String, TaskError> {
        self.0.typed("charge", &ctx, &cents).await
    }

    #[task]
    pub async fn hold(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("hold", &ctx, &()).await
    }

    #[task]
    pub async fn flaky(self: Arc<Self>, ctx: TaskContext, n: u32) -> Result<u32, TaskError> {
        self.0.typed("flaky", &ctx, &n).await
    }
}

/// A `#[tasks]` type with its own state.
#[derive(Default)]
pub struct Inventory {
    pub stock: Mutex<HashMap<String, u32>>,
}

#[tasks]
impl Inventory {
    #[task(name = "inventory.reserve")]
    pub async fn reserve(self: Arc<Self>, sku: String) -> Result<u32, TaskError> {
        let mut stock = lock(&self.stock);
        let held = stock.entry(sku).or_insert(0);
        *held += 1;
        Ok(*held)
    }
}

/// Grinds for a while on its thread.
#[task(blocking)]
pub fn grind(millis: u64) -> Result<u64, TaskError> {
    std::thread::sleep(Duration::from_millis(millis));
    Ok(millis)
}

pub fn defaults(probe: &Probe) {
    probe.default_output("reserve", |_| json!(3));
    probe.default_output("charge", |cents| json!(format!("ch-{cents}")));
    probe.default_output("hold", |_| json!(null));
    probe.default_output("flaky", |n| n.clone());
}

// ---- workflows ----

#[workflow(namespace = "calls", name = "one", version = "1")]
pub struct One;

#[workflow_methods]
impl One {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        let qty = ctx.call(Calls::reserve, &"A-1".to_string()).await?;
        Ok(qty)
    }
}

#[workflow(namespace = "calls", name = "chain", version = "1")]
pub struct Chain;

#[workflow_methods]
impl Chain {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, sku: String) -> WorkflowResult<String> {
        let qty = ctx.call(Calls::reserve, &sku).await?;
        let charge_id = ctx.call(Calls::charge, &(u64::from(qty) * 500)).await?;
        Ok(charge_id)
    }
}

#[workflow(namespace = "calls", name = "charge-only", version = "1")]
pub struct ChargeOnly;

#[workflow_methods]
impl ChargeOnly {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(ctx.call(Calls::charge, &500).await?)
    }
}

#[workflow(namespace = "calls", name = "repeat", version = "1")]
pub struct Repeat;

#[workflow_methods]
impl Repeat {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<u32>> {
        let mut out = Vec::new();
        for sku in ["A-1", "B-2", "A-1"] {
            out.push(ctx.call(Calls::reserve, &sku.to_string()).await?);
        }
        Ok(out)
    }
}

#[workflow(namespace = "calls", name = "then-block", version = "1")]
pub struct ThenBlock;

#[workflow_methods]
impl ThenBlock {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        let qty = ctx.call(Calls::reserve, &"A-1".to_string()).await?;
        let charge = ctx.call(Calls::charge, &u64::from(qty)).await?;
        ctx.call(Calls::hold, &()).await?;
        Ok(charge)
    }
}

#[workflow(namespace = "calls", name = "shared-state", version = "1")]
pub struct SharedState;

#[workflow_methods]
impl SharedState {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u32> {
        ctx.call(Inventory::reserve, &"apple".to_string()).await?;
        Ok(ctx.call(Inventory::reserve, &"apple".to_string()).await?)
    }
}

#[workflow(namespace = "calls", name = "grinder", version = "1")]
pub struct Grinder;

#[workflow_methods]
impl Grinder {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<u64> {
        Ok(ctx.call(Grind, &300).await?)
    }
}

#[workflow(namespace = "calls", name = "napper", version = "1")]
pub struct Napper;

#[workflow_methods]
impl Napper {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.sleep(10.millis()).await?;
        Ok(())
    }
}

#[workflow(namespace = "calls", name = "options", version = "1")]
pub struct Options;

#[workflow_methods]
impl Options {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Value> {
        let first = ctx
            .call(Calls::flaky, &1)
            .retry(Retry::exponential(5))
            .await;
        let second = ctx.call(Calls::flaky, &2).await;
        Ok(json!({
            "first": first.map_err(|e| e.to_string()),
            "second": second.map_err(|e| e.to_string()),
        }))
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<One>("calls.one"),
        Fixture::of::<Chain>("calls.chain").input(json!("A-1")),
        Fixture::of::<ChargeOnly>("calls.charge-only"),
        Fixture::of::<Repeat>("calls.repeat"),
        Fixture::of::<ThenBlock>("calls.then-block"),
        Fixture::of::<SharedState>("calls.shared-state"),
        Fixture::of::<Grinder>("calls.grinder"),
        Fixture::of::<Napper>("calls.napper"),
        Fixture::of::<Options>("calls.options"),
    ]
}

/// Docstrings in the feature that select a fixture.
pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Ok(qty)", "calls.one"),
        ("let charge_id = ctx.call(Charge", "calls.chain"),
    ]
}

// ---- steps ----

#[given("the tasks:")]
fn the_tasks(_world: &mut LibWorld) {
    // The tasks in the docstring are `Calls::reserve` and `Calls::charge`, registered on every engine.
}

#[then(regex = r#"^the handle resolves to the "u32" the task returned$"#)]
async fn resolves_to_u32(world: &mut LibWorld) {
    let last = world.last();
    let outcome = world.outcome(last).await;
    assert_eq!(outcome.value(), &json!(3));
    assert!(outcome.value().is_u64());
}

#[then(regex = r#"^"(\w+)" received the cents computed from "(\w+)"'s output$"#)]
async fn received_cents(world: &mut LibWorld, charge: String, reserve: String) {
    let last = world.last();
    world.outcome(last).await;
    let qty = world.probe.calls_of(&reserve)[0].input.clone();
    assert_eq!(qty, json!("A-1"));
    assert_eq!(world.probe.calls_of(&charge)[0].input, json!(3 * 500));
}

#[given(regex = r#"^a workflow that calls "Charge"$"#)]
fn calls_charge(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.charge-only"));
}

#[then(
    regex = r#"^the task's context reports the instance id, the task name "(\w+)" and attempt 1$"#
)]
async fn context_reports(world: &mut LibWorld, name: String) {
    let last = world.last();
    world.outcome(last).await;
    let call = &world.probe.calls_of(&name)[0];
    assert_eq!(call.instance, world.runs[last].id);
    assert_eq!(call.name, name);
    assert_eq!(call.attempt, 1);
}

#[then("a non-empty idempotency key")]
fn non_empty_key(world: &mut LibWorld) {
    assert!(!world.probe.calls_of("charge")[0].key.is_empty());
}

#[given(regex = r#"^a workflow that calls "Reserve" for "A-1", then "B-2", then "A-1" again$"#)]
fn calls_reserve_three(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.repeat"));
}

#[then("the journal holds 3 distinct steps")]
async fn journal_three(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await;
    let id = world.runs[last].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    let completed = history
        .iter()
        .filter(|e| matches!(e, Event::TaskCompleted { task_name, .. } if task_name == "reserve"))
        .count();
    assert_eq!(completed, 3);
}

#[given(regex = r#"^a completed instance that called "Reserve" and "Charge"$"#)]
async fn completed_chain(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.chain"));
    world.start("it", json!("A-1")).await;
    let last = world.last();
    world.outcome(last).await;
    world.mark = Some(world.probe.calls().len());
}

#[when(regex = r#"^a new engine on the same store replays it with "Engine::replay_all"$"#)]
async fn new_engine_replays(world: &mut LibWorld) {
    world.crash().await;
    let engine = world.engine();
    world.divergences = Some(engine.replay_all().await.expect("replay_all"));
}

#[then("neither task runs")]
fn neither_runs(world: &mut LibWorld) {
    assert_eq!(Some(world.probe.calls().len()), world.mark);
}

#[given(
    regex = r#"^a workflow that calls "Reserve", then "Charge", then a task that blocks until released$"#
)]
fn then_blocks(world: &mut LibWorld) {
    world.probe.script("hold", |s| s.block = true);
    world.set_fixture(super::fixture("calls.then-block"));
}

#[when("the engine is dropped while the blocking task runs")]
async fn dropped_while_blocking(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the blocking task runs", || probe.running("hold"))
        .await;
    world.crash().await;
}

#[then(regex = r#"^"(\w+)" and "(\w+)" each ran once in total$"#)]
async fn each_once(world: &mut LibWorld, a: String, b: String) {
    world.probe.release("hold");
    let last = world.last();
    world.outcome(last).await;
    assert_eq!(world.probe.count(&a), 1, "{a} calls");
    assert_eq!(world.probe.count(&b), 1, "{b} calls");
}

#[then("the blocking task runs again")]
async fn blocking_again(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.count("hold"), 2);
    assert_eq!(world.probe.completed("hold"), 1);
}

#[when(regex = r#"^I build an engine that registers the workflow but not "(\w+)"$"#)]
fn build_without(world: &mut LibWorld, _task: String) {
    let fixture = world.fixture();
    world.no_tasks = true;
    world.only = Some(vec![fixture.register.clone()]);
    world.build_error = world.builder().build().err().map(|e| match e {
        Error::Invalid(message) => format!("Invalid: {message}"),
        other => format!("other: {other}"),
    });
}

#[then(regex = r#"^"build" fails with "Error::Invalid" naming "([^"]+)"$"#)]
fn build_fails_invalid(world: &mut LibWorld, name: String) {
    let error = world.build_error.clone().expect("build failed");
    assert!(error.starts_with("Invalid:"), "{error}");
    assert!(error.contains(&name), "`{error}` should name `{name}`");
}

#[given(regex = r#"^a "\#\[tasks\]" type "Inventory" holding a shared stock map$"#)]
fn inventory_type(_world: &mut LibWorld) {}

#[given(regex = r#"^a workflow that calls "Inventory::reserve" twice for the same sku$"#)]
fn inventory_twice(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.shared-state"));
}

#[then("the shared stock map shows both reservations")]
async fn both_reservations(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!(2));
    let inventory = world.harnesses.inventory();
    assert_eq!(lock(&inventory.stock).get("apple"), Some(&2));
}

#[given(regex = r#"^a "\#\[task\(blocking\)\]" function that sleeps its thread for 300 ms$"#)]
fn blocking_fn(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.grinder"));
}

#[given("a second workflow that only waits on a 10 ms step")]
fn second_waits(_world: &mut LibWorld) {}

#[when("I execute both at the same time")]
async fn execute_both(world: &mut LibWorld) {
    world
        .start_fixture("grinder", super::fixture("calls.grinder"), json!(null))
        .await;
    world
        .start_fixture("napper", super::fixture("calls.napper"), json!(null))
        .await;
}

#[then("the second workflow completes first")]
async fn second_first(world: &mut LibWorld) {
    let napper = world.run_index("napper");
    world.outcome(napper).await.value();
    let grinder = world.run_index("grinder");
    let napped: Instant = world.runs[napper].finished_at.expect("finished");
    world.outcome(grinder).await.value();
    let ground = world.runs[grinder].finished_at.expect("finished");
    assert!(napped < ground);
}

#[given(
    regex = r#"^a workflow that calls the same task once with "\.retry\(Retry::exponential\(5\)\)" and once with no options$"#
)]
fn same_task_options(world: &mut LibWorld) {
    world.set_fixture(super::fixture("calls.options"));
}

#[when("the task fails retryably on its first attempt both times")]
async fn fails_first_attempt(world: &mut LibWorld) {
    world.probe.script("flaky", |s| s.fail_first_attempt = true);
    world.start("it", json!(null)).await;
}

#[then("the first step succeeds on attempt 2")]
async fn first_attempt_two(world: &mut LibWorld) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    assert_eq!(value["first"], json!({"Ok": 1}));
    let first: Vec<u32> = world
        .probe
        .calls_of("flaky")
        .iter()
        .filter(|c| c.input == json!(1))
        .map(|c| c.attempt)
        .collect();
    assert_eq!(first, vec![1, 2]);
}

#[then("the second step fails after 1 attempt")]
async fn second_fails_once(world: &mut LibWorld) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    assert!(value["second"].get("Err").is_some(), "{value}");
    let second = world
        .probe
        .calls_of("flaky")
        .iter()
        .filter(|c| c.input == json!(2))
        .count();
    assert_eq!(second, 1);
}
