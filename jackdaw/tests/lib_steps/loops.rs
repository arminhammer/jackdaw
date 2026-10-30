//! tasks/loops.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Lines {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum ExportStatus {
    Running,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Batch {
    pub count: u32,
    #[serde(default)]
    pub concurrency: Option<usize>,
}

pub struct Looping(pub Arc<Probe>);

#[tasks]
impl Looping {
    #[task(name = "loops.price")]
    pub async fn price(self: Arc<Self>, ctx: TaskContext, line: String) -> Result<u64, TaskError> {
        self.0.typed("price", &ctx, &line).await
    }
    #[task(name = "loops.check_export")]
    pub async fn check_export(
        self: Arc<Self>,
        ctx: TaskContext,
        job: String,
    ) -> Result<ExportStatus, TaskError> {
        self.0.typed("check_export", &ctx, &job).await
    }
    #[task(name = "loops.step")]
    pub async fn step(self: Arc<Self>, ctx: TaskContext, item: u32) -> Result<String, TaskError> {
        self.0.typed("step", &ctx, &item).await
    }
    #[task(name = "loops.import")]
    pub async fn import(self: Arc<Self>, ctx: TaskContext, row: u32) -> Result<(), TaskError> {
        self.0.typed("import", &ctx, &row).await
    }
    #[task(name = "loops.task")]
    pub async fn task(self: Arc<Self>, ctx: TaskContext, item: u32) -> Result<u32, TaskError> {
        self.0.typed("task", &ctx, &item).await
    }
    #[task(name = "loops.tick")]
    pub async fn tick(self: Arc<Self>, ctx: TaskContext, item: u32) -> Result<u32, TaskError> {
        self.0.typed("tick", &ctx, &item).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("price", |line| {
        json!(line.as_str().map_or(0, |l| l.len() as u64 * 100))
    });
    probe.default_output("check_export", |_| json!("Done"));
    probe.default_output("step", |_| json!("ok"));
    probe.default_output("import", |_| json!(null));
    probe.default_output("task", |item| item.clone());
    probe.default_output("tick", |item| item.clone());
}

#[workflow(namespace = "loops", name = "for", version = "1")]
pub struct ForLines;

#[workflow_methods]
impl ForLines {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: Lines) -> WorkflowResult<Vec<u64>> {
        let mut prices = Vec::new();
        for line in &order.lines {
            prices.push(ctx.call(Looping::price, line).await?);
        }
        Ok(prices)
    }
}

#[workflow(namespace = "loops", name = "while", version = "1")]
pub struct Poll;

#[workflow_methods]
impl Poll {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, job: String) -> WorkflowResult<ExportStatus> {
        let mut status = ctx.call(Looping::check_export, &job).await?;
        while status != ExportStatus::Done {
            ctx.sleep(30.secs()).await?;
            status = ctx.call(Looping::check_export, &job).await?;
        }
        Ok(status)
    }
}

#[workflow(namespace = "loops", name = "break-continue", version = "1")]
pub struct BreakContinue;

#[workflow_methods]
impl BreakContinue {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<u32>> {
        let mut seen = Vec::new();
        for item in 0..10u32 {
            if item % 2 == 1 {
                continue;
            }
            seen.push(item);
            if ctx.call(Looping::step, &item).await? == "stop" {
                break;
            }
        }
        Ok(seen)
    }
}

#[workflow(namespace = "loops", name = "fail-fast", version = "1")]
pub struct FailFast;

#[workflow_methods]
impl FailFast {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        for item in 1..=5u32 {
            ctx.call(Looping::step, &item).await?;
        }
        Ok(())
    }
}

#[workflow(namespace = "loops", name = "collect", version = "1")]
pub struct Collect;

#[workflow_methods]
impl Collect {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Value> {
        let (mut ok, mut failed) = (0, Vec::new());
        for row in 1..=5u32 {
            match ctx.call(Looping::import, &row).await {
                Ok(()) => ok += 1,
                Err(e) => failed.push(e.to_string()),
            }
        }
        Ok(json!({ "imported": ok, "failed": failed.len() }))
    }
}

#[workflow(namespace = "loops", name = "for-each", version = "1")]
pub struct Each;

#[workflow_methods]
impl Each {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, batch: Batch) -> WorkflowResult<Vec<u32>> {
        let items: Vec<u32> = (0..batch.count).collect();
        Ok(match batch.concurrency {
            Some(limit) => {
                ctx.for_each(&items, Looping::task)
                    .concurrency(limit)
                    .await?
            }
            None => ctx.for_each(&items, Looping::task).await?,
        })
    }
}

#[workflow(namespace = "loops", name = "count", version = "1")]
pub struct Count;

#[workflow_methods]
impl Count {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, n: u32) -> WorkflowResult<u32> {
        let mut total = 0;
        for item in 1..=n {
            total += ctx.call(Looping::tick, &item).await?;
        }
        Ok(total)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Pages {
    pub cursor: u32,
    pub total: u32,
}

#[workflow(namespace = "loops", name = "pager", version = "1")]
pub struct Pager;

#[workflow_methods]
impl Pager {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, pages: Pages) -> WorkflowResult<u32> {
        let mut cursor = pages.cursor;
        loop {
            ctx.call(Looping::tick, &cursor).await?;
            cursor += 1;
            if cursor == pages.total {
                return Ok(cursor);
            }
            if cursor.is_multiple_of(500) {
                return ctx.continue_as_new(Pages {
                    cursor,
                    total: pages.total,
                });
            }
        }
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Pager>("loops.pager"),
        Fixture::of::<ForLines>("loops.for"),
        Fixture::of::<Poll>("loops.while").input(json!("export-1")),
        Fixture::of::<BreakContinue>("loops.break-continue"),
        Fixture::of::<FailFast>("loops.fail-fast"),
        Fixture::of::<Collect>("loops.collect"),
        Fixture::of::<Each>("loops.for-each"),
        Fixture::of::<Count>("loops.count").input(json!(100)),
    ]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![
        ("prices.push(ctx.call(Price, line)", "loops.for"),
        ("while status != Status::Done", "loops.while"),
    ]
}

fn lines(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("line-{i}")).collect()
}

#[when(regex = r"^I execute it with (\d+) lines$")]
async fn execute_lines(world: &mut LibWorld, n: usize) {
    world.start("it", json!({ "lines": lines(n) })).await;
}

#[then(regex = r#"^"price" ran (\d+) times in line order$"#)]
async fn price_in_order(world: &mut LibWorld, n: usize) {
    world.settle().await;
    let inputs: Vec<Value> = world
        .probe
        .calls_of("price")
        .into_iter()
        .map(|c| c.input)
        .collect();
    assert_eq!(
        inputs,
        lines(n).into_iter().map(Value::from).collect::<Vec<_>>()
    );
}

#[then(regex = r"^the result has (\d+) prices in line order$")]
async fn prices_in_order(world: &mut LibWorld, n: usize) {
    let last = world.last();
    let value = world.outcome(last).await.value().clone();
    let expected: Vec<u64> = lines(n).iter().map(|l| l.len() as u64 * 100).collect();
    assert_eq!(value, json!(expected));
}

#[given(regex = r#"^"CheckExport" returns "Running" twice and then "Done"$"#)]
fn running_twice(world: &mut LibWorld) {
    world.probe.script("check_export", |s| {
        s.outputs = [json!("Running"), json!("Running"), json!("Done")].into();
    });
}

#[when("I execute it on a virtual clock")]
async fn execute_virtual(world: &mut LibWorld) {
    let input = world.fixture().input;
    world.start("it", input).await;
}

#[then(regex = r"^(\d+) seconds of virtual time passed$")]
fn virtual_seconds(world: &mut LibWorld, seconds: u64) {
    assert_eq!(world.clock.elapsed(), Duration::from_secs(seconds));
}

#[given(
    regex = r#"^a workflow that loops over 10 items, skips odd items with "continue", and breaks when a step returns "stop"$"#
)]
fn break_continue(world: &mut LibWorld) {
    world.set_fixture(super::fixture("loops.break-continue"));
}

#[given(regex = r#"^the step returns "stop" for item (\d+)$"#)]
fn stop_for(world: &mut LibWorld, item: u32) {
    world.probe.script("step", |s| {
        s.map = Some(Arc::new(move |input| {
            json!(if *input == json!(item) { "stop" } else { "ok" })
        }));
    });
}

#[then(regex = r"^the step ran for items (.+) only$")]
async fn step_ran_for(world: &mut LibWorld, list: String) {
    world.settle().await;
    let expected: Vec<Value> = list
        .replace(" and ", ", ")
        .split(',')
        .map(|n| json!(n.trim().parse::<u32>().unwrap()))
        .collect();
    let inputs: Vec<Value> = world
        .probe
        .calls_of("step")
        .into_iter()
        .map(|c| c.input)
        .collect();
    assert_eq!(inputs, expected);
}

#[given(regex = r#"^a "for" loop over 5 items whose step fails non-retryably on item (\d+)$"#)]
fn fails_on_item(world: &mut LibWorld, item: u32) {
    world.probe.script("step", |s| {
        s.fail_on
            .push((json!(item), FaultSpec::non_retryable("bad item"), None));
    });
    world.set_fixture(super::fixture("loops.fail-fast"));
}

#[then("the workflow fails with that fault")]
async fn fails_with_that(world: &mut LibWorld) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.detail.as_deref(), Some("bad item"));
}

#[then(regex = r"^items (\d+) and (\d+) were never processed$")]
fn never_processed(world: &mut LibWorld, a: u32, b: u32) {
    let inputs: Vec<Value> = world
        .probe
        .calls_of("step")
        .into_iter()
        .map(|c| c.input)
        .collect();
    assert!(
        !inputs.contains(&json!(a)) && !inputs.contains(&json!(b)),
        "{inputs:?}"
    );
}

#[given(regex = r#"^a loop that does "match ctx\.call\(Import, &row\).+"$"#)]
fn collect_loop(world: &mut LibWorld) {
    world.set_fixture(super::fixture("loops.collect"));
}

#[given(regex = r"^rows (\d+) and (\d+) of 5 fail$")]
fn rows_fail(world: &mut LibWorld, a: u32, b: u32) {
    world.probe.script("import", |s| {
        for row in [a, b] {
            s.fail_on
                .push((json!(row), FaultSpec::non_retryable("bad row"), None));
        }
    });
}

#[then(regex = r"^the result reports (\d+) imported and (\d+) failed$")]
async fn reports(world: &mut LibWorld, ok: u32, failed: u32) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({"imported": ok, "failed": failed})
    );
}

#[given("a task that records how many copies of it run at once")]
fn records_copies(world: &mut LibWorld) {
    world
        .probe
        .script("task", |s| s.delay = Some(Duration::from_secs(1)));
}

#[when(
    regex = r#"^a workflow calls "ctx\.for_each\(&items, Task\)(?:\.concurrency\((\d+)\))?" over (\d+) items$"#
)]
async fn for_each_over(world: &mut LibWorld, limit: String, count: u32) {
    world
        .probe
        .script("task", |s| s.delay = Some(Duration::from_secs(1)));
    let concurrency: Option<usize> = limit.parse().ok();
    world
        .start_fixture(
            "it",
            super::fixture("loops.for-each"),
            json!({"count": count, "concurrency": concurrency}),
        )
        .await;
}

#[then(regex = r"^(\d+) results come back in input order$")]
async fn results_in_order(world: &mut LibWorld, n: u32) {
    let last = world.last();
    let expected: Vec<u32> = (0..n).collect();
    assert_eq!(world.outcome(last).await.value(), &json!(expected));
}

#[then(regex = r"^at most (\d+) cop(?:y|ies) ran at once$")]
async fn at_most(world: &mut LibWorld, n: usize) {
    world.settle().await;
    let peak = world.probe.max_in_flight("task");
    assert!(peak <= n, "{peak} ran at once");
    assert_eq!(peak, n, "the limit should be used");
}

#[given(regex = r#"^a "for" loop over 100 items that calls a counting task$"#)]
fn count_loop(world: &mut LibWorld) {
    world.set_fixture(super::fixture("loops.count"));
}

#[when(regex = r"^the engine is dropped during iteration (\d+)$")]
async fn dropped_during(world: &mut LibWorld, item: u32) {
    world
        .probe
        .script("tick", |s| s.block_on = Some(json!(item)));
    world.start("it", json!(100)).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the iteration runs", || probe.running("tick"))
        .await;
    world.crash().await;
    world.probe.release("tick");
}

#[then("the counting task ran exactly once for each item")]
async fn once_per_item(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!((1..=100).sum::<u32>())
    );
    for item in 1..=100u32 {
        let completed = world
            .probe
            .calls_of("tick")
            .iter()
            .filter(|c| c.input == json!(item) && !c.cancelled)
            .count();
        assert_eq!(completed, 1, "item {item}");
    }
}

#[given(regex = r#"^"ctx\.for_each\(&items, Task\)\.concurrency\(5\)" over 50 items$"#)]
fn for_each_fifty(world: &mut LibWorld) {
    world.probe.script("task", |s| s.block_on = Some(json!(23)));
}

#[when(regex = r"^the engine is dropped after (\d+) items finished$")]
async fn dropped_after_items(world: &mut LibWorld, n: usize) {
    world
        .start_fixture(
            "it",
            super::fixture("loops.for-each"),
            json!({"count": 50, "concurrency": 5}),
        )
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("enough items finished", || probe.completed("task") >= n)
        .await;
    // The other items keep finishing around the blocked one until the loop can go no further. Drop the engine only
    // then, once every finished item's result is in the store (not merely returned by its task).
    loop {
        let returned = world.probe.completed("task");
        super::durability::completions_durable(world, "loops.task", returned).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        if world.probe.completed("task") == returned {
            break;
        }
    }
    world.crash().await;
    world.probe.release("task");
}

#[then("each item's task ran exactly once in total")]
async fn each_item_once(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    for item in 0..50u32 {
        let completed = world
            .probe
            .calls_of("task")
            .iter()
            .filter(|c| c.input == json!(item) && !c.cancelled)
            .count();
        assert_eq!(completed, 1, "item {item}");
    }
}

#[given(
    regex = r#"^a "loop" that processes one page per iteration and calls "ctx\.continue_as_new\(cursor\)" every 500 pages$"#
)]
fn pager(world: &mut LibWorld) {
    world.set_fixture(super::fixture("loops.pager"));
}

#[when(regex = r"^it processes (\d+) pages$")]
async fn processes_pages(world: &mut LibWorld, total: u32) {
    world
        .start("it", json!({ "cursor": 0, "total": total }))
        .await;
    let last = world.last();
    // One step per page: on a network store that is a few thousand round trips.
    let outcome = world.outcome_within(last, Duration::from_secs(300)).await;
    assert_eq!(outcome.value(), &json!(total));
}

#[then(regex = r"^no single run's journal holds more than (\d+) iterations$")]
async fn runs_bounded(world: &mut LibWorld, limit: usize) {
    let id = world.runs[world.last()].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    let mut in_run = 0;
    let mut runs = 1;
    for event in history {
        match event {
            Event::TaskCompleted { .. } => {
                in_run += 1;
                assert!(in_run <= limit, "run {runs} holds more than {limit} steps");
            }
            Event::ContinuedAsNew { run, .. } => {
                runs = run;
                in_run = 0;
            }
            _ => {}
        }
    }
    assert_eq!(runs, 4, "2000 pages in runs of 500");
}

#[then("all runs share the instance id")]
fn runs_share_id(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    assert!(
        world
            .probe
            .calls_of("tick")
            .iter()
            .all(|call| call.instance == id)
    );
}
