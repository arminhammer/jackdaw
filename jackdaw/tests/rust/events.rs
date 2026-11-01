//! Observing a run: a concrete `EventStream` per handle, ordered events, a stream that ends after the terminal
//! event, and events and the result usable at the same time.
use crate::support::shop::*;
use crate::support::*;
use jackdaw::testing::TestEngine;

#[workflow(namespace = "events", name = "checkout", version = "1.0.0")]
pub struct Checkout;

#[workflow_methods]
impl Checkout {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Shipment> {
        let receipt = ctx.call(Shop::charge, &input).await?;
        Ok(ctx.call(Shop::ship, &receipt).await?)
    }
}

fn engine(shop: &Arc<Shop>) -> TestEngine {
    TestEngine::builder()
        .tasks(shop.clone())
        .workflow::<Checkout>()
        .build()
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_successful_run_emits_its_events_in_order() -> TestResult {
    let engine = engine(&Shop::new());

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let events = collect(run.events()).await;
    run.await?;

    assert_eq!(
        names(&events),
        [
            "WorkflowStarted",
            "TaskStarted",
            "TaskCompleted",
            "TaskStarted",
            "TaskCompleted",
            "WorkflowCompleted",
        ]
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn events_name_the_instance_and_the_task() -> TestResult {
    let engine = engine(&Shop::new());

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let id = run.id().to_string();
    let events = collect(run.events()).await;
    run.await?;

    let completed: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::TaskCompleted {
                task_name,
                instance_id,
                ..
            } => {
                assert_eq!(instance_id, &id);
                Some(task_name.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(completed, ["charge", "ship"]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_failed_run_emits_task_faulted_then_workflow_failed() -> TestResult {
    let engine = engine(&Shop::failing_charges(500));

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let events = collect(run.events()).await;
    let result = run.await;

    assert!(matches!(result, Err(Error::Faulted(_))));
    let names = names(&events);
    assert_eq!(names.last(), Some(&"WorkflowFailed"));
    assert!(names.contains(&"TaskFaulted"));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn the_stream_ends_after_the_terminal_event() -> TestResult {
    let engine = engine(&Shop::new());

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let mut events = run.events();
    run.await?;
    while events.next().await.is_some() {}

    assert!(events.next().await.is_none(), "an ended stream stays ended");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn events_and_the_result_can_be_used_at_the_same_time() -> TestResult {
    let engine = engine(&Shop::new());

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let watcher = tokio::spawn(collect(run.events()));
    let shipment = run.await?;
    let events = watcher.await?;

    assert_eq!(shipment.order_id, "o-1");
    assert_eq!(names(&events).last(), Some(&"WorkflowCompleted"));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn the_stream_can_be_subscribed_more_than_once() -> TestResult {
    let engine = engine(&Shop::new());

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let first = tokio::spawn(collect(run.events()));
    let second = tokio::spawn(collect(run.events()));
    run.await?;

    assert_eq!(names(&first.await?), names(&second.await?));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn history_returns_the_same_events_after_the_run() -> TestResult {
    let engine = engine(&Shop::new());

    let run = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?;
    let id = run.id().to_string();
    let live = collect(run.events()).await;
    run.await?;

    let stored = engine.history(&id).await?;
    assert_eq!(names(&live), names(&stored));
    Ok(())
}
