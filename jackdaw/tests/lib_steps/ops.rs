//! operations/instance_management.feature and operations/lifecycle.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::gherkin::Step as GherkinStep;
use cucumber::{given, then, when};
use jackdaw::api::{ListFilter, Status};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub fail: bool,
}

pub mod shop {
    use super::*;
    use crate::lib_steps::durability::Durable;

    /// `shop/order@1.0.0` for the listing scenarios: waits for "go" or fails when asked to.
    #[workflow(namespace = "shop", name = "order", version = "1.0.0")]
    pub struct Order;

    #[workflow_methods]
    impl Order {
        #[run]
        async fn run(ctx: &mut WorkflowContext<Self>, plan: Plan) -> WorkflowResult<String> {
            ctx.call(Durable::a, &()).await?;
            if plan.wait {
                let _: Value = ctx.wait_for_event("go").await?;
            }
            if plan.fail {
                return Err(TaskError::fault("validation").title("refused").into());
            }
            Ok("done".into())
        }
    }

    #[workflow(namespace = "shop", name = "refund", version = "1.0.0")]
    pub struct Refund;

    #[workflow_methods]
    impl Refund {
        #[run]
        async fn run(_ctx: &mut WorkflowContext<Self>, plan: Plan) -> WorkflowResult<String> {
            if plan.fail {
                return Err(TaskError::fault("validation").title("refused").into());
            }
            Ok("refunded".into())
        }
    }
}

pub fn variants() -> Vec<Fixture> {
    vec![
        Fixture::of::<shop::Order>("ops.order"),
        Fixture::of::<shop::Refund>("ops.refund"),
    ]
}

/// The listing scenarios register their own `shop/order` and `shop/refund`.
fn use_shop(world: &mut LibWorld) {
    world.only = Some(vec![
        super::variant("ops.order").register,
        super::variant("ops.refund").register,
    ]);
}

async fn start_with_id(
    world: &mut LibWorld,
    label: &str,
    fixture: Fixture,
    id: &str,
    input: Value,
) -> Result<(), Error> {
    let engine = world.engine();
    let handle = (fixture.start_with_id)(engine, id.to_string(), input).await?;
    let now = world.clock.now();
    world.push_run(label, fixture, handle, now);
    Ok(())
}

// ---- execute_with_id and handles ----

#[when(
    regex = r#"^I call "Engine::execute_with_id\(\\"([\w-]+)\\", OrderWorkflow::run, order\)"$"#
)]
async fn execute_with_id(world: &mut LibWorld, id: String) {
    use_shop(world);
    start_with_id(world, "it", super::variant("ops.order"), &id, json!({}))
        .await
        .expect("starts");
}

#[then(regex = r#"^the handle's id is "([\w-]+)"$"#)]
fn handle_id(world: &mut LibWorld, id: String) {
    assert_eq!(world.runs[world.last()].id, id);
}

#[then(regex = r#"^"Engine::status\(\\"([\w-]+)\\"\)" is "Running" or "Completed"$"#)]
async fn running_or_completed(world: &mut LibWorld, id: String) {
    let status = world.engine().status(&id).await.expect("status");
    assert!(
        matches!(status, Status::Running | Status::Completed),
        "{status:?}"
    );
}

#[given(regex = r#"^I started "([\w-]+)" (?:with "execute_with_id"|as "OrderWorkflow")$"#)]
async fn started(world: &mut LibWorld, id: String) {
    use_shop(world);
    start_with_id(world, "first", super::variant("ops.order"), &id, json!({}))
        .await
        .expect("starts");
    let first = world.run_index("first");
    world.outcome(first).await.value();
}

#[when(regex = r#"^I call "execute_with_id\(\\"([\w-]+)\\", \.\.\)" again with the same input$"#)]
async fn again(world: &mut LibWorld, id: String) {
    start_with_id(world, "second", super::variant("ops.order"), &id, json!({}))
        .await
        .expect("starts");
}

#[then("no second instance is created")]
async fn no_second(world: &mut LibWorld) {
    let listed = world.engine().list(&ListFilter::new()).await.expect("list");
    assert_eq!(listed.len(), 1, "{listed:?}");
}

#[then(regex = r#"^the returned handle refers to the existing "([\w-]+)"$"#)]
async fn refers_to_existing(world: &mut LibWorld, id: String) {
    let second = world.run_index("second");
    assert_eq!(world.runs[second].id, id);
    assert_eq!(world.outcome(second).await.value(), &json!("done"));
}

#[then("the workflow's tasks ran once")]
fn tasks_once(world: &mut LibWorld) {
    assert_eq!(world.probe.count("a"), 1);
}

#[when(regex = r#"^I call "execute_with_id\(\\"([\w-]+)\\", RefundWorkflow::run, \.\.\)"$"#)]
async fn as_refund(world: &mut LibWorld, id: String) {
    let error = start_with_id(
        world,
        "refund",
        super::variant("ops.refund"),
        &id,
        json!({}),
    )
    .await
    .expect_err("a different workflow");
    world
        .notes
        .insert("error".into(), json!(format!("{error:?}")));
}

#[then(regex = r#"^it fails with "Error::Invalid" naming the existing workflow$"#)]
fn invalid_naming_existing(world: &mut LibWorld) {
    let error = world.notes["error"].as_str().expect("an error").to_string();
    assert!(
        error.starts_with("Invalid(") && error.contains("shop/order@1.0.0"),
        "{error}"
    );
}

#[when(regex = r#"^(\d+) tasks call "execute_with_id\(\\"([\w-]+)\\", \.\.\)" at the same time$"#)]
async fn concurrent_starts(world: &mut LibWorld, n: usize, id: String) {
    use_shop(world);
    let engine = world.engine();
    let fixture = super::variant("ops.order");
    let starts = (0..n).map(|_| {
        let (engine, fixture, id) = (engine.clone(), fixture.clone(), id.clone());
        tokio::spawn(async move { (fixture.start_with_id)(engine, id, json!({})).await })
    });
    let handles: Vec<ExecutionHandle<Value>> = futures::future::join_all(starts)
        .await
        .into_iter()
        .map(|joined| joined.expect("joined").expect("started"))
        .collect();
    let now = world.clock.now();
    for (i, handle) in handles.into_iter().enumerate() {
        world.push_run(&format!("start{i}"), fixture.clone(), handle, now);
    }
}

#[then(regex = r#"^exactly one instance "([\w-]+)" exists$"#)]
async fn exactly_one(world: &mut LibWorld, id: String) {
    let listed = world.engine().list(&ListFilter::new()).await.expect("list");
    assert_eq!(listed.iter().filter(|info| info.id == id).count(), 1);
    assert_eq!(listed.len(), 1);
}

#[then(regex = r"^all (\d+) handles resolve to the same result$")]
async fn same_results(world: &mut LibWorld, n: usize) {
    world.settle().await;
    assert_eq!(world.runs.len(), n);
    assert!(world.runs.iter().all(|run| {
        run.outcome
            .as_ref()
            .is_some_and(|o| o.value() == &json!("done"))
    }));
    assert_eq!(world.probe.count("a"), 1);
}

#[given(regex = r#"^a running "Approval" instance started by another part of the program$"#)]
async fn approval_elsewhere(world: &mut LibWorld) {
    let engine = world.engine();
    let handle = (super::fixture("api.approval").start)(engine, json!(null))
        .await
        .expect("starts");
    world.notes.insert("elsewhere".into(), json!(handle.id()));
}

#[when(regex = r#"^I call "Engine::handle::<Approval>\(id\)"$"#)]
async fn handle_approval(world: &mut LibWorld) {
    let id = world.notes["elsewhere"]
        .as_str()
        .expect("an id")
        .to_string();
    let handle = world
        .engine()
        .handle::<super::api::Approval>(&id)
        .await
        .expect("a handle");
    let now = world.clock.now();
    world.push_run("found", super::fixture("api.approval"), handle.erase(), now);
}

#[then("I can signal, query, cancel and await it through that handle")]
async fn use_found(world: &mut LibWorld) {
    let handle = world.handle(world.run_index("found"));
    assert!(
        !handle
            .query(super::api::Approval::is_approved)
            .await
            .expect("query")
    );
    handle
        .signal(super::api::Approval::approve, ())
        .await
        .expect("signal");
    assert_eq!(handle.clone().await.expect("completes"), json!("approved"));
    handle
        .cancel()
        .await
        .expect("cancel is accepted for a finished instance");
}

#[given("a completed instance in the store")]
async fn completed_in_store(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("calls.chain"), json!("A-1"))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
    world.mark = Some(world.probe.calls().len());
}

#[when(regex = r#"^a new engine calls "Engine::handle::<W>\(id\)" and awaits it$"#)]
async fn fresh_handle(world: &mut LibWorld) {
    world.crash().await;
    let id = world.runs[world.last()].id.clone();
    let handle = (super::fixture("calls.chain").attach)(world.engine(), id)
        .await
        .expect("attach");
    world
        .notes
        .insert("stored".into(), handle.await.expect("the stored result"));
}

#[then("it returns the stored result without running anything")]
fn stored_result(world: &mut LibWorld) {
    assert_eq!(world.notes["stored"], json!("ch-1500"));
    assert_eq!(Some(world.probe.calls().len()), world.mark);
}

#[when(
    regex = r#"^I call "Engine::handle::<RefundWorkflow>\(id\)" for an "OrderWorkflow" instance$"#
)]
async fn wrong_handle(world: &mut LibWorld) {
    use_shop(world);
    start_with_id(world, "it", super::variant("ops.order"), "o-1", json!({}))
        .await
        .expect("starts");
    let error = (super::variant("ops.refund").attach)(world.engine(), "o-1".into())
        .await
        .expect_err("the wrong workflow");
    world
        .notes
        .insert("error".into(), json!(format!("{error:?}")));
}

#[given(regex = r"^a running instance waiting on a 1-hour sleep$")]
async fn waiting_hour(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("timers.nap"), json!(1))
        .await;
    world.quiesce().await;
}

#[when(regex = r#"^I call "Engine::status\(\\"does-not-exist\\"\)"$"#)]
async fn status_unknown(world: &mut LibWorld) {
    let error = world
        .engine()
        .status("does-not-exist")
        .await
        .expect_err("unknown");
    world.notes.insert("error".into(), json!(error.to_string()));
}

#[then("it fails with an error that says the instance was not found")]
fn not_found(world: &mut LibWorld) {
    assert!(
        world.notes["error"]
            .as_str()
            .expect("an error")
            .contains("not found")
    );
}

// ---- listing ----

#[given("these instances in the store:")]
async fn these_instances(world: &mut LibWorld, step: &GherkinStep) {
    use_shop(world);
    let rows = step.table.as_ref().expect("a table").rows.clone();
    for row in rows.iter().skip(1) {
        let (id, workflow, status, started) = (&row[0], &row[1], &row[2], &row[3]);
        world.clock.advance_to(started.parse().expect("a time"));
        let fixture = if workflow.contains("refund") {
            "ops.refund"
        } else {
            "ops.order"
        };
        let plan = json!({ "wait": status == "Running", "fail": status == "Faulted" });
        start_with_id(world, id, super::variant(fixture), id, plan)
            .await
            .expect("starts");
        world.quiesce().await;
    }
}

async fn list(world: &mut LibWorld, filter: ListFilter) {
    let ids: Vec<String> = world
        .engine()
        .list(&filter)
        .await
        .expect("list")
        .into_iter()
        .map(|i| i.id)
        .collect();
    world.notes.insert("listed".into(), json!(ids));
}

#[when(regex = r#"^I call "Engine::list" with status "(\w+)"$"#)]
async fn list_status(world: &mut LibWorld, status: String) {
    let status = match status.as_str() {
        "Running" => Status::Running,
        "Completed" => Status::Completed,
        "Faulted" => Status::Faulted,
        _ => Status::Cancelled,
    };
    list(world, ListFilter::new().status(status)).await;
}

#[when(regex = r#"^I call "Engine::list" with workflow "([\w/]+)"$"#)]
async fn list_workflow(world: &mut LibWorld, workflow: String) {
    list(world, ListFilter::new().workflow(workflow)).await;
}

#[when(regex = r#"^I call "Engine::list" with started after "(.+)"$"#)]
async fn list_after(world: &mut LibWorld, after: String) {
    let after: chrono::DateTime<chrono::Utc> = after.parse().expect("a time");
    list(
        world,
        ListFilter::new().started_after(after - chrono::Duration::seconds(1)),
    )
    .await;
}

#[then(regex = r#"^I get "([\w-]+)"(?: and "([\w-]+)")?$"#)]
fn i_get(world: &mut LibWorld, first: String, second: String) {
    let mut expected = vec![first];
    if !second.is_empty() {
        expected.push(second);
    }
    assert_eq!(world.notes["listed"], json!(expected));
}

#[given(regex = r"^(\d+) instances in the store$")]
async fn many_instances(world: &mut LibWorld, n: usize) {
    if n >= 1000 {
        // The index scenario measures what one more start writes.
        super::durability::record_writes(world);
    }
    let engine = world.engine();
    let mut handles = Vec::new();
    for _ in 0..n {
        let start = super::fixture("api.double").start;
        handles.push(start(engine.clone(), json!(1)).await.expect("starts"));
    }
    for handle in handles {
        handle.await.expect("finishes");
    }
}

#[when(regex = r"^I list them with a page size of (\d+)$")]
async fn paged(world: &mut LibWorld, size: usize) {
    let engine = world.engine();
    let mut pages = Vec::new();
    let mut offset = 0;
    loop {
        let page = engine
            .list(&ListFilter::new().offset(offset).limit(size))
            .await
            .expect("list");
        if page.is_empty() {
            break;
        }
        offset += page.len();
        pages.push(page.into_iter().map(|info| info.id).collect::<Vec<_>>());
    }
    world.notes.insert("pages".into(), json!(pages));
}

#[then(regex = r"^I get 3 pages with 100, 100 and 50 instances$")]
fn three_pages(world: &mut LibWorld) {
    let sizes: Vec<usize> = world.notes["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|p| p.as_array().map_or(0, Vec::len))
        .collect();
    assert_eq!(sizes, vec![100, 100, 50]);
}

#[then("no instance appears twice")]
fn no_duplicates(world: &mut LibWorld) {
    let all: Vec<String> = world.notes["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .flat_map(|page| page.as_array().cloned().unwrap_or_default())
        .filter_map(|id| id.as_str().map(str::to_string))
        .collect();
    let unique: std::collections::HashSet<&String> = all.iter().collect();
    assert_eq!(unique.len(), all.len());
}

// ---- cancellation and purging ----

#[given("a running instance that was cancelled")]
async fn cancelled_running(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("timers.nap"), json!(1))
        .await;
    world.quiesce().await;
    world.handle(world.last()).cancel().await.expect("cancel");
    let last = world.last();
    world.outcome(last).await;
}

#[given("a parent waiting on a running child")]
async fn parent_and_child(world: &mut LibWorld) {
    world.probe.script("dur_second", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.parent"), json!(7))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the child runs", || probe.running("dur_second"))
        .await;
}

#[when("the parent is cancelled")]
async fn parent_cancelled(world: &mut LibWorld) {
    world.handle(world.last()).cancel().await.expect("cancel");
    let last = world.last();
    world.outcome(last).await;
}

#[then(regex = r#"^the child's status becomes "(\w+)"$"#)]
async fn child_status(world: &mut LibWorld, status: String) {
    let child = world.probe.calls_of("dur_second")[0].instance.clone();
    let engine = world.engine();
    let mut actual = engine.status(&child).await.expect("status");
    for _ in 0..200 {
        if format!("{actual:?}") == status {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        actual = engine.status(&child).await.expect("status");
    }
    assert_eq!(format!("{actual:?}"), status);
}

#[given(regex = r#"^an engine built with "retention\((\d+)\.days\(\)\)"$"#)]
fn retention(world: &mut LibWorld, days: u64) {
    world.extras.push(Arc::new(move |builder: EngineBuilder| {
        builder.retention(days.days())
    }));
}

#[given("a completed instance that finished 8 days ago")]
async fn old_instance(world: &mut LibWorld) {
    // The running instance of this scenario is started first, 30 days before the sweep.
    world
        .start_fixture("running", super::fixture("timers.nap"), json!(24 * 365))
        .await;
    world.pass(Duration::from_secs(22 * 86_400)).await;
    world
        .start_fixture("old", super::fixture("api.double"), json!(1))
        .await;
    let old = world.run_index("old");
    world.outcome_frozen(old).await.value();
    world.pass(Duration::from_secs(7 * 86_400)).await;
}

#[given("a completed instance that finished 1 day ago")]
async fn recent_instance(world: &mut LibWorld) {
    world
        .start_fixture("recent", super::fixture("api.double"), json!(2))
        .await;
    let recent = world.run_index("recent");
    world.outcome_frozen(recent).await.value();
    world.pass(Duration::from_secs(86_400)).await;
}

#[given("a running instance that started 30 days ago")]
fn running_thirty(world: &mut LibWorld) {
    assert!(world.runs.iter().any(|run| run.label == "running"));
}

#[when("the retention sweep runs")]
async fn sweep(world: &mut LibWorld) {
    let purged = world.engine().sweep().await.expect("sweep");
    world.notes.insert("purged".into(), json!(purged));
}

#[then("the 8-day-old instance and its history are removed from the store")]
async fn old_removed(world: &mut LibWorld) {
    let old = world.runs[world.run_index("old")].id.clone();
    assert_eq!(world.notes["purged"], json!([old]));
    let engine = world.engine();
    assert!(engine.status(&old).await.is_err());
    assert!(engine.history(&old).await.expect("history").is_empty());
}

#[then("the other two instances remain")]
async fn others_remain(world: &mut LibWorld) {
    let engine = world.engine();
    for label in ["recent", "running"] {
        let id = world.runs[world.run_index(label)].id.clone();
        engine.status(&id).await.expect("still there");
    }
}

#[given("a completed instance")]
async fn a_completed_instance(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("api.double"), json!(3))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[given("a running instance")]
async fn a_running_instance(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("timers.nap"), json!(1))
        .await;
    world.quiesce().await;
}

#[when(regex = r#"^I call "Engine::purge\(id\)"$"#)]
async fn purge(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let result = world.engine().purge(&id).await;
    world
        .notes
        .insert("purge".into(), json!(result.err().map(|e| e.to_string())));
}

#[then(regex = r#"^"Engine::status\(id\)" reports the instance as not found$"#)]
async fn purged_not_found(world: &mut LibWorld) {
    assert_eq!(world.notes["purge"], Value::Null);
    let id = world.runs[world.last()].id.clone();
    let error = world.engine().status(&id).await.expect_err("purged");
    assert!(error.to_string().contains("not found"));
}

#[then("it fails with an error that says the instance is still running")]
fn still_running(world: &mut LibWorld) {
    let error = world.notes["purge"].as_str().expect("refused");
    assert!(error.contains("still running"), "{error}");
}

#[when("I start one more")]
async fn start_one_more(world: &mut LibWorld) {
    let store = super::durability::store_writes(world);
    let before = store.len();
    world
        .start_fixture("more", super::fixture("api.double"), json!(5))
        .await;
    let index = world.run_index("more");
    world.outcome(index).await.value();
    let after = super::durability::store_writes(world);
    world.notes.insert(
        "start-writes".into(),
        json!(after[before..].iter().map(|(_, n)| *n).collect::<Vec<_>>()),
    );
    world.notes.insert(
        "first-writes".into(),
        json!(
            after[..6.min(after.len())]
                .iter()
                .map(|(_, n)| *n)
                .collect::<Vec<_>>()
        ),
    );
}

#[then("the store write for the start is independent of the number of existing instances")]
fn start_independent(world: &mut LibWorld) {
    let largest = world.notes["start-writes"]
        .as_array()
        .expect("writes")
        .iter()
        .filter_map(Value::as_u64)
        .max()
        .unwrap_or(0);
    assert!(
        largest < 4_096,
        "starting an instance wrote {largest} bytes at once"
    );
}

// ---- lifecycle ----

#[given(regex = r"^an engine with a Redb store running a task that takes (\d+) seconds$")]
async fn running_slow_task(world: &mut LibWorld, seconds: u64) {
    super::common::durable(world).await;
    world
        .probe
        .script("slow", |s| s.delay = Some(Duration::from_secs(seconds)));
    world
        .start_fixture("it", super::fixture("lifecycle.slow-then-nap"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until_frozen("the task runs", || probe.running("slow"))
        .await;
}

/// Shuts the engine down while driving the test clock, like a real clock would move.
async fn shutdown(world: &mut LibWorld, grace: Duration) -> Duration {
    let engine = world.engine();
    let started = world.clock.now();
    let shutting = tokio::spawn(async move { engine.shutdown(grace).await });
    while !shutting.is_finished() {
        tokio::time::sleep(Duration::from_millis(10)).await;
        // Time moves only for the task in flight: once it has returned, its result is being written, and moving the
        // clock to the grace deadline then would end the shutdown early.
        if world.probe.completed("slow") == 0 {
            world.clock.advance_next(None);
        }
    }
    shutting.await.expect("shutdown");
    (world.clock.now() - started).to_std().unwrap_or_default()
}

#[when(regex = r#"^I call "Engine::shutdown\(\)" with a grace period of (\d+) seconds?$"#)]
async fn shutdown_with_grace(world: &mut LibWorld, seconds: u64) {
    let took = shutdown(world, Duration::from_secs(seconds)).await;
    world
        .notes
        .insert("shutdown".into(), json!(took.as_secs_f64()));
}

#[then("the task finishes and its result is persisted")]
async fn finished_and_persisted(world: &mut LibWorld) {
    assert_eq!(world.probe.completed("slow"), 1);
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert!(details.steps >= 1, "{details:?}");
}

#[then(regex = r#"^"shutdown" returns after the task completes$"#)]
fn after_task(world: &mut LibWorld) {
    let took = world.notes["shutdown"].as_f64().expect("timed");
    assert!(
        (2.0..10.0).contains(&took),
        "shutdown took {took}s of engine time"
    );
}

#[then(regex = r#"^the instance is left "Running" for the next process to recover$"#)]
async fn left_running(world: &mut LibWorld) {
    world.crash().await;
    let id = world.runs[world.last()].id.clone();
    let status = world.engine().status(&id).await.expect("status");
    assert_eq!(format!("{status:?}"), "Running");
}

#[given("an engine that is shutting down")]
async fn shutting_down(world: &mut LibWorld) {
    shutdown(world, Duration::from_secs(1)).await;
}

#[when(regex = r#"^I call "execute"$"#)]
async fn call_execute(world: &mut LibWorld) {
    let error = (super::fixture("api.double").start)(world.engine(), json!(1))
        .await
        .expect_err("refused");
    world.notes.insert("error".into(), json!(error.to_string()));
}

#[then("it fails with an error that says the engine is shutting down")]
fn shutting_down_error(world: &mut LibWorld) {
    assert!(
        world.notes["error"]
            .as_str()
            .expect("an error")
            .contains("shutting down")
    );
}

#[given("an instance sleeping for 1 hour")]
async fn sleeping_hour(world: &mut LibWorld) {
    super::common::durable(world).await;
    world
        .start_fixture("it", super::fixture("timers.nap"), json!(1))
        .await;
    world.quiesce().await;
}

#[when(regex = r#"^I call "Engine::shutdown\(\)"$"#)]
async fn shutdown_default(world: &mut LibWorld) {
    let started = Instant::now();
    let took = shutdown(world, Duration::from_secs(30)).await;
    world
        .notes
        .insert("shutdown".into(), json!(took.as_secs_f64()));
    world.notes.insert(
        "shutdown-real".into(),
        json!(started.elapsed().as_secs_f64()),
    );
}

#[then("it returns promptly")]
fn promptly(world: &mut LibWorld) {
    assert_eq!(world.notes["shutdown"], json!(0.0));
}

#[then("the instance resumes with the remaining sleep in the next process")]
async fn resumes_remaining(world: &mut LibWorld) {
    world.crash().await;
    let last = world.last();
    world.resume(last).await;
    let id = world.runs[last].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert_eq!(details.timers.len(), 1);
    world.outcome(last).await.value();
}

#[given("a task that takes 1 minute")]
async fn minute_task(world: &mut LibWorld) {
    super::common::durable(world).await;
    world
        .probe
        .script("slow", |s| s.delay = Some(Duration::from_secs(60)));
    world
        .start_fixture("it", super::fixture("lifecycle.slow-then-nap"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until_frozen("the task runs", || probe.running("slow"))
        .await;
}

#[then(regex = r#"^"shutdown" returns after about 1 second$"#)]
fn about_one_second(world: &mut LibWorld) {
    assert_eq!(world.notes["shutdown"], json!(1.0));
}

#[then("the step is not recorded as completed")]
async fn not_recorded(world: &mut LibWorld) {
    world.crash().await;
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert_eq!(details.steps, 0, "{details:?}");
}

#[then("the next process runs the task again with the same idempotency key")]
async fn runs_again_same_key(world: &mut LibWorld) {
    world.probe.script("slow", |s| s.delay = None);
    let last = world.last();
    world.resume(last).await;
    let probe = Arc::clone(&world.probe);
    world
        .until("the task runs again", || probe.count("slow") >= 2)
        .await;
    let calls = world.probe.calls_of("slow");
    assert_eq!(calls[0].key, calls[1].key);
}

#[when("a step completes and the engine is dropped immediately")]
async fn step_then_drop(world: &mut LibWorld) {
    world.probe.script("b", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("dur.abc"), json!(null))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until_frozen("a completed", || probe.completed("a") >= 1)
        .await;
    world.crash().await;
    world.probe.release_all();
}

#[then("a new engine on the same path sees the step as completed")]
async fn sees_step(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert!(details.steps >= 1, "{details:?}");
}

pub struct Lifecycle(pub Arc<Probe>);

#[tasks]
impl Lifecycle {
    #[task(name = "lifecycle.slow")]
    pub async fn slow(self: Arc<Self>, ctx: TaskContext, _: ()) -> Result<(), TaskError> {
        self.0.typed("slow", &ctx, &()).await
    }
}

#[workflow(namespace = "lifecycle", name = "slow-then-nap", version = "1")]
pub struct SlowThenNap;

#[workflow_methods]
impl SlowThenNap {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.call(Lifecycle::slow, &()).await?;
        ctx.sleep(1.hours()).await?;
        Ok(())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![Fixture::of::<SlowThenNap>("lifecycle.slow-then-nap")]
}
