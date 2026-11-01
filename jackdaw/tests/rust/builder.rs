//! Building an engine: defaults, durable stores, and registration mistakes caught up front.
use crate::support::shop::*;
use crate::support::*;

#[test]
fn builds_with_defaults() {
    // In-memory persistence and cache, no tasks, no workflows.
    Engine::builder().build().unwrap();
}

#[test]
fn builds_with_a_durable_store() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Redb::open(dir.path().join("jackdaw.db"))?;

    Engine::builder().store(store).build()?;
    Ok(())
}

// A task may not take the name of a built-in call type; otherwise `call: http` would silently change meaning.
#[task(name = "http")]
async fn fake_http(input: Name) -> Result<Greeting, TaskError> {
    Ok(Greeting::hello(&input.name))
}

#[test]
fn rejects_a_task_that_shadows_a_builtin_call_type() {
    let err = Engine::builder().task(FakeHttp).build().unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(err.to_string().contains("http"));
}

#[test]
fn rejects_duplicate_task_names() {
    let err = Engine::builder()
        .task(Greet)
        .task(Greet)
        .build()
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
}

#[workflow(namespace = "builder-tests", name = "uses-charge", version = "1.0.0")]
pub struct UsesCharge;

#[workflow_methods]
impl UsesCharge {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Receipt> {
        Ok(ctx.call(Shop::charge, &input).await?)
    }
}

#[test]
fn rejects_a_workflow_that_calls_an_unregistered_task() {
    // `UsesCharge` calls `Shop::charge`, which was never registered. This is found when the engine is built,
    // not in the middle of a run.
    let err = Engine::builder()
        .workflow::<UsesCharge>()
        .build()
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(err.to_string().contains("charge"));
}

#[test]
fn rejects_the_same_workflow_version_registered_twice() {
    let err = Engine::builder()
        .tasks(Shop::new())
        .workflow::<UsesCharge>()
        .workflow::<UsesCharge>()
        .build()
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
}

#[workflow(namespace = "builder-tests", name = "three-steps", version = "1.0.0")]
pub struct ThreeSteps;

#[workflow_methods]
impl ThreeSteps {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Shipment> {
        ctx.call(Shop::audit, &input).await?;
        let receipt = ctx.call(Shop::charge, &input).await?;
        Ok(ctx.call(Shop::ship, &receipt).await?)
    }
}

#[tokio::test]
async fn a_tiny_event_buffer_never_deadlocks_a_run_nobody_is_watching() -> TestResult {
    let engine = Engine::builder()
        .tasks(Shop::new())
        .workflow::<ThreeSteps>()
        .event_buffer_size(1)
        .build()?;

    let shipment = engine
        .execute(ThreeSteps::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert_eq!(shipment.order_id, "o-1");
    Ok(())
}
