//! Durability: a workflow survives the process that was running it, and code changes between deploys are either
//! handled explicitly or refused. Rust workflows are journaled step by step; after a restart `#[run]` runs again
//! from the top, finished steps return their recorded result, and the first unfinished step runs live.
//!
//! A "crash" here is dropping the engine mid-workflow: nothing is cancelled and no terminal event is written, so
//! the instance can be resumed by a new engine over the same store. Events are emitted after they are persisted.
use crate::support::shop::*;
use crate::support::*;
use jackdaw::testing::TestEngine;
use std::path::Path;

// ---------------------------------------------------------------------------------------------------------------
// The workflow under test, in several "deploys". All but `v2` share one identity: replay/order 1.0.0.
// ---------------------------------------------------------------------------------------------------------------

/// Charge, wait for an approval event, then ship.
mod v1 {
    use super::*;

    #[workflow(namespace = "replay", name = "order", version = "1.0.0")]
    pub struct OrderFlow;

    #[workflow_methods]
    impl OrderFlow {
        #[run]
        pub async fn run(
            ctx: &mut WorkflowContext<Self>,
            input: Order,
        ) -> WorkflowResult<Shipment> {
            let receipt = ctx.call(Shop::charge, &input).await?;
            let _approved: Approved = ctx.wait_for_event("approved").await?;
            Ok(ctx.call(Shop::ship, &receipt).await?)
        }
    }
}

/// Same identity, different code: waits for approval *before* charging. Incompatible with v1 histories.
mod reordered {
    use super::*;

    #[workflow(namespace = "replay", name = "order", version = "1.0.0")]
    pub struct OrderFlow;

    #[workflow_methods]
    impl OrderFlow {
        #[run]
        pub async fn run(
            ctx: &mut WorkflowContext<Self>,
            input: Order,
        ) -> WorkflowResult<Shipment> {
            let _approved: Approved = ctx.wait_for_event("approved").await?;
            let receipt = ctx.call(Shop::charge, &input).await?;
            Ok(ctx.call(Shop::ship, &receipt).await?)
        }
    }
}

/// A new version of the workflow: an audit step first. Old instances stay on v1's code.
mod v2 {
    use super::*;

    #[workflow(namespace = "replay", name = "order", version = "2.0.0")]
    pub struct OrderFlow;

    #[workflow_methods]
    impl OrderFlow {
        #[run]
        pub async fn run(
            ctx: &mut WorkflowContext<Self>,
            input: Order,
        ) -> WorkflowResult<Shipment> {
            ctx.call(Shop::audit, &input).await?;
            let receipt = ctx.call(Shop::charge, &input).await?;
            let _approved: Approved = ctx.wait_for_event("approved").await?;
            Ok(ctx.call(Shop::ship, &receipt).await?)
        }
    }
}

/// An in-place change under the same version, made safe with `ctx.patched`.
mod patched {
    use super::*;

    #[workflow(namespace = "replay", name = "order", version = "1.0.0")]
    pub struct OrderFlow;

    #[workflow_methods]
    impl OrderFlow {
        #[run]
        pub async fn run(
            ctx: &mut WorkflowContext<Self>,
            input: Order,
        ) -> WorkflowResult<Shipment> {
            if ctx.patched("audit-before-charge") {
                ctx.call(Shop::audit, &input).await?;
            }
            let receipt = ctx.call(Shop::charge, &input).await?;
            let _approved: Approved = ctx.wait_for_event("approved").await?;
            Ok(ctx.call(Shop::ship, &receipt).await?)
        }
    }
}

/// Lenient replay: steps are matched by task name and ordinal, so an inserted step does not break old histories.
mod lenient {
    use super::*;

    #[workflow(
        namespace = "replay",
        name = "order",
        version = "1.0.0",
        replay = "lenient"
    )]
    pub struct OrderFlow;

    #[workflow_methods]
    impl OrderFlow {
        #[run]
        pub async fn run(
            ctx: &mut WorkflowContext<Self>,
            input: Order,
        ) -> WorkflowResult<Shipment> {
            ctx.call(Shop::notify, &input).await?;
            let receipt = ctx.call(Shop::charge, &input).await?;
            let _approved: Approved = ctx.wait_for_event("approved").await?;
            Ok(ctx.call(Shop::ship, &receipt).await?)
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------------------------

fn store(dir: &Path) -> Redb {
    Redb::open(dir.join("jackdaw.db")).unwrap()
}

/// Starts v1, lets it charge, then abandons the engine. Returns the instance id.
async fn crash_after_charge(dir: &Path, shop: &Arc<Shop>) -> String {
    let engine = Engine::builder()
        .store(store(dir))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .build()
        .unwrap();
    let run = engine
        .execute(v1::OrderFlow::run, Order::new("o-1", 500))
        .await
        .unwrap();
    let id = run.id().to_string();
    let mut events = run.events();
    until_task_completed(&mut events, "charge").await;
    id // the engine, handle and events drop here: the "crash"
}

fn approve() -> Approved {
    Approved { by: "ada".into() }
}

// ---------------------------------------------------------------------------------------------------------------
// Resume
// ---------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn resume_continues_where_the_process_stopped() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;
    assert_eq!(shop.charges.get(), 1);

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .build()?;
    let run = engine.resume::<v1::OrderFlow>(&id).await?;
    run.send_event("approved", approve()).await?;
    let shipment = run.await?;

    assert_eq!(shipment.order_id, "o-1");
    assert_eq!(
        shop.charges.get(),
        1,
        "the charge was not repeated after the restart"
    );
    assert_eq!(shop.ships.get(), 1);
    Ok(())
}

#[tokio::test]
async fn a_resumed_instance_has_each_finished_task_in_its_history_once() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .build()?;
    let run = engine.resume::<v1::OrderFlow>(&id).await?;
    run.send_event("approved", approve()).await?;
    run.await?;

    let history = engine.history(&id).await?;
    let charges = history
        .iter()
        .filter(|e| matches!(e, Event::TaskCompleted { task_name, .. } if task_name == "charge"))
        .count();
    assert_eq!(charges, 1);
    Ok(())
}

#[tokio::test]
async fn replayed_steps_are_not_reported_to_event_consumers_as_new_work() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .build()?;
    let run = engine.resume::<v1::OrderFlow>(&id).await?;
    let events = tokio::spawn(collect(run.events()));
    run.send_event("approved", approve()).await?;
    run.await?;
    let events = events.await?;

    let fresh_charge_work = events.iter().any(|e| {
        !e.replayed() && matches!(e, Event::TaskStarted { task_name, .. } if task_name == "charge")
    });
    assert!(
        !fresh_charge_work,
        "the charge step was replayed, not re-run: {:?}",
        names(&events)
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Changing code between deploys
// ---------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn reordering_steps_under_the_same_version_fails_the_resume() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<reordered::OrderFlow>()
        .build()?;
    let err = engine
        .resume::<reordered::OrderFlow>(&id)
        .await?
        .await
        .unwrap_err();

    let Error::NonDeterminism(_) = &err else {
        panic!("expected NonDeterminism, got {err:?}");
    };
    let message = err.to_string();
    assert!(message.contains("charge"), "names the step: {message}");
    assert!(
        message.contains("replay.rs"),
        "points at the code that diverged: {message}"
    );
    assert_eq!(
        shop.charges.get(),
        1,
        "nothing runs against a history it does not match"
    );
    Ok(())
}

#[tokio::test]
async fn an_instance_cannot_be_resumed_under_a_different_workflow_version() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .workflow::<v2::OrderFlow>()
        .build()?;
    let err = engine.resume::<v2::OrderFlow>(&id).await.unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(err.to_string().contains("1.0.0"));
    Ok(())
}

#[tokio::test]
async fn bumping_the_version_keeps_old_instances_on_old_code_and_sends_new_ones_to_new_code()
-> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let old_id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .workflow::<v2::OrderFlow>()
        .build()?;

    let old = engine.resume::<v1::OrderFlow>(&old_id).await?;
    old.send_event("approved", approve()).await?;
    old.await?;
    assert_eq!(
        shop.audits.get(),
        0,
        "the old instance never saw the new audit step"
    );

    let new = engine
        .execute(v2::OrderFlow::run, Order::new("o-2", 700))
        .await?;
    new.send_event("approved", approve()).await?;
    new.await?;
    assert_eq!(shop.audits.get(), 1, "new instances run the new code");
    Ok(())
}

#[tokio::test]
async fn patched_changes_only_apply_to_executions_that_started_after_the_change() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let old_id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<patched::OrderFlow>()
        .build()?;

    let old = engine.resume::<patched::OrderFlow>(&old_id).await?;
    old.send_event("approved", approve()).await?;
    old.await?;
    assert_eq!(
        shop.audits.get(),
        0,
        "`patched` is false when replaying an older history"
    );

    let new = engine
        .execute(patched::OrderFlow::run, Order::new("o-2", 700))
        .await?;
    new.send_event("approved", approve()).await?;
    new.await?;
    assert_eq!(
        shop.audits.get(),
        1,
        "`patched` is true for a fresh execution"
    );
    Ok(())
}

#[tokio::test]
async fn lenient_replay_tolerates_an_inserted_step() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<lenient::OrderFlow>()
        .build()?;
    let run = engine.resume::<lenient::OrderFlow>(&id).await?;
    run.send_event("approved", approve()).await?;
    run.await?;

    assert_eq!(
        shop.notices.get(),
        1,
        "the new step has no record, so it ran live"
    );
    assert_eq!(
        shop.charges.get(),
        1,
        "the recorded charge was not repeated"
    );
    Ok(())
}

#[tokio::test]
async fn replay_all_reports_stored_histories_that_no_longer_match_the_code() -> TestResult {
    let dir = tempfile::tempdir()?;
    let shop = Shop::new();
    let id = crash_after_charge(dir.path(), &shop).await;

    let compatible = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<v1::OrderFlow>()
        .build()?;
    assert!(compatible.replay_all().await?.is_empty());
    drop(compatible);

    let changed = Engine::builder()
        .store(store(dir.path()))
        .tasks(shop.clone())
        .workflow::<reordered::OrderFlow>()
        .build()?;
    let divergences = changed.replay_all().await?;

    assert_eq!(divergences.len(), 1);
    assert_eq!(divergences[0].instance_id, id);
    assert!(divergences[0].to_string().contains("charge"));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Clock, randomness and ids are journaled values
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub now: String,
    pub random: f64,
    pub uuid: String,
}

#[derive(Default)]
pub struct Recorder {
    pub observed: std::sync::Mutex<Vec<Snapshot>>,
}

#[tasks]
impl Recorder {
    #[task]
    pub async fn observe(self: Arc<Self>, snapshot: Snapshot) -> Result<(), TaskError> {
        self.observed.lock().unwrap().push(snapshot);
        Ok(())
    }
}

#[workflow(namespace = "replay", name = "stamped", version = "1.0.0")]
pub struct Stamped;

#[workflow_methods]
impl Stamped {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _input: Name) -> WorkflowResult<Snapshot> {
        // Direct clock, randomness and UUID calls are compile errors in `#[run]`; these are the journaled ones.
        let snapshot = Snapshot {
            now: ctx.now().to_rfc3339(),
            random: ctx.random(),
            uuid: ctx.uuid().to_string(),
        };
        ctx.call(Recorder::observe, &snapshot).await?;
        let _approved: Approved = ctx.wait_for_event("approved").await?;
        Ok(snapshot)
    }
}

#[tokio::test]
async fn now_random_and_uuid_have_the_same_values_after_a_restart() -> TestResult {
    let dir = tempfile::tempdir()?;
    let recorder = Arc::new(Recorder::default());

    let id = {
        let engine = Engine::builder()
            .store(store(dir.path()))
            .tasks(recorder.clone())
            .workflow::<Stamped>()
            .build()?;
        let run = engine.execute(Stamped::run, Name::new("Ada")).await?;
        let id = run.id().to_string();
        let mut events = run.events();
        until_task_completed(&mut events, "observe").await;
        id
    };

    let engine = Engine::builder()
        .store(store(dir.path()))
        .tasks(recorder.clone())
        .workflow::<Stamped>()
        .build()?;
    let run = engine.resume::<Stamped>(&id).await?;
    run.send_event("approved", approve()).await?;
    let final_snapshot = run.await?;

    let observed = recorder.observed.lock().unwrap();
    assert_eq!(
        observed.len(),
        1,
        "`observe` ran once; the replay used its record"
    );
    assert_eq!(
        final_snapshot, observed[0],
        "replay reproduced the original values"
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Tests catch non-determinism for you: TestEngine replays every workflow from its own history
// ---------------------------------------------------------------------------------------------------------------

static RUNS: AtomicUsize = AtomicUsize::new(0);

#[workflow(namespace = "replay", name = "unstable", version = "1.0.0")]
pub struct Unstable;

#[workflow_methods]
impl Unstable {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<()> {
        // Behaviour depends on hidden global state: exactly what the compiler cannot see and replay must not allow.
        #[allow_nondeterminism]
        let first_time = RUNS.fetch_add(1, Ordering::SeqCst) == 0;
        if first_time {
            ctx.call(Shop::charge, &input).await?;
        } else {
            ctx.call(Shop::audit, &input).await?;
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn test_engine_fails_a_workflow_that_does_not_replay_to_the_same_steps() {
    let engine = TestEngine::builder()
        .tasks(Shop::new())
        .workflow::<Unstable>()
        .build()
        .unwrap();

    let err = engine
        .execute(Unstable::run, Order::new("o-1", 500))
        .await
        .unwrap()
        .await
        .unwrap_err();

    assert!(matches!(err, Error::NonDeterminism(_)), "got {err:?}");
}
