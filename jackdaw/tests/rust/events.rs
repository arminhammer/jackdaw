//! Observing a run: a concrete `EventStream` per handle, ordered events, a stream that ends after the terminal
//! event, and events and the result usable at the same time.
use crate::support::shop::*;
use crate::support::*;
use jackdaw::testing::TestEngine;
use serde_json::{Value, json};

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

/// Records the `jackdaw::yaml` trace events: their `event` and `line` fields.
#[derive(Clone, Default)]
struct YamlTrace(Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for YamlTrace {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Fields(String, String);
        impl tracing::field::Visit for Fields {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                match field.name() {
                    "event" => self.0 = value.to_string(),
                    "line" => self.1 = value.to_string(),
                    _ => {}
                }
            }
            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        }
        if event.metadata().target() == "jackdaw::yaml" {
            let mut fields = Fields(String::new(), String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push((fields.0, fields.1));
        }
    }
}

// The library never prints: what a YAML script task writes is reported as `tracing` events, for the application's
// subscriber to show or not.
#[tokio::test]
async fn a_yaml_scripts_output_is_a_trace_event() -> TestResult {
    use tracing::instrument::WithSubscriber as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    let trace = YamlTrace::default();
    let subscriber = tracing_subscriber::registry().with(trace.clone());
    let workflow = Workflow::<Value, Value>::from_yaml(
        r#"
document:
  dsl: '1.0.0'
  namespace: events
  name: say-hello
  version: '1.0.0'
do:
  - hello:
      run:
        shell:
          command: echo hello from the script
"#,
    )?;
    async {
        let engine = Engine::builder().build()?;
        engine.execute(&workflow, json!({})).await?.await?;
        TestResult::Ok(())
    }
    .with_subscriber(subscriber)
    .await?;

    let events = trace.0.lock().unwrap().clone();
    assert!(
        events.contains(&(
            "script_line".to_string(),
            "hello from the script".to_string()
        )),
        "{events:?}"
    );
    assert!(
        events.iter().any(|(event, _)| event == "task_start"),
        "{events:?}"
    );
    Ok(())
}
