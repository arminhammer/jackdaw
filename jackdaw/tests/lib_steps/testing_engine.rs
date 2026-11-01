//! interop/testing_engine.feature: fakes and simulated crashes on the `TestEngine`.
use super::world::LibWorld;
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use jackdaw::testing::TestEngine;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Tasks that count how often their real code runs.
#[derive(Default)]
pub struct Counted {
    pub charges: AtomicUsize,
    pub steps: [AtomicUsize; 3],
}

#[tasks]
impl Counted {
    #[task(name = "te.charge")]
    pub async fn charge(
        self: Arc<Self>,
        _ctx: TaskContext,
        order: String,
    ) -> Result<String, TaskError> {
        self.charges.fetch_add(1, Ordering::SeqCst);
        Ok(format!("charged {order}"))
    }

    #[task(name = "te.step")]
    pub async fn step(self: Arc<Self>, _ctx: TaskContext, n: usize) -> Result<usize, TaskError> {
        self.steps[n - 1].fetch_add(1, Ordering::SeqCst);
        Ok(n)
    }
}

#[workflow(namespace = "te", name = "checkout", version = "1")]
pub struct Checkout;

#[workflow_methods]
impl Checkout {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, order: String) -> WorkflowResult<String> {
        Ok(ctx.call(Counted::charge, &order).await?)
    }
}

#[workflow(namespace = "te", name = "three-steps", version = "1")]
pub struct ThreeSteps;

#[workflow_methods]
impl ThreeSteps {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _input: ()) -> WorkflowResult<usize> {
        let mut total = 0;
        for n in 1..=3 {
            total += ctx.call(Counted::step, &n).await?;
        }
        Ok(total)
    }
}

/// Runs `test` on its own paused runtime, the way `#[tokio::test(start_paused = true)]` would.
fn paused<T: Send + 'static>(test: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(test)
        .join()
        .expect("the test engine thread")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .expect("a runtime")
}

#[given(regex = r#"^a "TestEngine" built with "replace_task\(Charge, .+\)"$"#)]
fn with_fake(world: &mut LibWorld) {
    world.notes.insert("fake".into(), json!(true));
}

#[when(regex = r#"^a workflow that calls "Charge" runs$"#)]
fn charge_runs(world: &mut LibWorld) {
    let (output, real_calls) = paused(|| {
        runtime().block_on(async {
            let counted = Arc::new(Counted::default());
            let engine = TestEngine::builder()
                .tasks(Arc::clone(&counted))
                .workflow::<Checkout>()
                .replace_task(Counted::charge, |_order: String| async {
                    Ok("fake".to_string())
                })
                .build()
                .expect("builds");
            let output = engine
                .execute(Checkout::run, "order-1".to_string())
                .await
                .expect("starts")
                .await
                .expect("finishes");
            (output, counted.charges.load(Ordering::SeqCst))
        })
    });
    world.notes.insert("te-output".into(), json!(output));
    world.notes.insert("real".into(), json!(real_calls));
}

#[then(regex = r#"^the real "charge" function is not called$"#)]
fn real_not_called(world: &mut LibWorld) {
    assert_eq!(world.notes["real"], json!(0));
}

#[given(regex = r#"^a "TestEngine" with a workflow of three steps$"#)]
fn three_steps(world: &mut LibWorld) {
    world.notes.insert("three".into(), json!(true));
}

#[when(regex = r#"^I call "TestEngine::crash_after_step\((\d+)\)" and execute it$"#)]
fn crash_after(world: &mut LibWorld, steps: usize) {
    world.notes.insert("crash-after".into(), json!(steps));
}

#[when(regex = r#"^I call "TestEngine::restart\(\)"$"#)]
fn restart(world: &mut LibWorld) {
    let steps = world.notes["crash-after"].as_u64().expect("a step count") as usize;
    let (crashed, recovered, output, counts) = paused(move || {
        runtime().block_on(async move {
            let counted = Arc::new(Counted::default());
            let engine = TestEngine::builder()
                .tasks(Arc::clone(&counted))
                .workflow::<ThreeSteps>()
                .build()
                .expect("builds");
            engine.crash_after_step(steps);
            let handle = engine
                .execute_with_id("three", ThreeSteps::run, ())
                .await
                .expect("starts");
            let crashed = handle
                .await
                .expect_err("the crash stops the run")
                .to_string();
            let recovery = engine.restart().await.expect("restarts");
            let output = engine
                .handle::<ThreeSteps>("three")
                .await
                .expect("a handle")
                .await
                .expect("finishes");
            let counts: Vec<usize> = counted
                .steps
                .iter()
                .map(|n| n.load(Ordering::SeqCst))
                .collect();
            (crashed, recovery.resumed, output, counts)
        })
    });
    world.notes.insert("crashed".into(), json!(crashed));
    world.notes.insert("recovered".into(), json!(recovered));
    world.notes.insert("te-output".into(), json!(output));
    world.notes.insert("counts".into(), json!(counts));
}

#[then(regex = r"^steps 1 and 2 are not re-executed$")]
fn not_reexecuted(world: &mut LibWorld) {
    assert!(
        world.notes["crashed"]
            .as_str()
            .expect("an error")
            .contains("stopped")
    );
    assert_eq!(world.notes["recovered"], json!(["three"]));
    assert_eq!(world.notes["te-output"], json!(6));
    assert_eq!(world.notes["counts"], json!([1, 1, 1]));
}

#[workflow(namespace = "te", name = "week-apart", version = "1")]
pub struct WeekApart;

#[workflow_methods]
impl WeekApart {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _input: ()) -> WorkflowResult<usize> {
        let first = ctx.call(Counted::step, &1).await?;
        ctx.sleep(7.days()).await?;
        Ok(first + ctx.call(Counted::step, &2).await?)
    }
}

#[workflow(namespace = "te", name = "reads-the-clock", version = "1")]
pub struct ReadsTheClock;

#[workflow_methods]
impl ReadsTheClock {
    #[run]
    pub async fn run(_ctx: &mut WorkflowContext<Self>, _input: ()) -> WorkflowResult<u64> {
        #[allow_nondeterminism]
        let now = std::time::SystemTime::now();
        Ok(now
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64))
    }
}

/// A task tested as a plain function.
#[task(name = "te.charge-order")]
pub async fn charge(order: String) -> Result<String, TaskError> {
    Ok(format!("charged {order}"))
}

#[given(regex = r#"^a "TestEngine" with a workflow that sleeps 7 days between two tasks$"#)]
fn week_apart(world: &mut LibWorld) {
    world.notes.insert("te-run".into(), json!("week-apart"));
}

#[given(regex = r#"^a workflow that reads "SystemTime::now" under "\#\[allow_nondeterminism\]"$"#)]
fn reads_clock(world: &mut LibWorld) {
    world
        .notes
        .insert("te-run".into(), json!("reads-the-clock"));
}

#[given(regex = r#"^a "TestEngine" built with "self_replay\(false\)"$"#)]
fn no_self_replay(world: &mut LibWorld) {
    world.notes.insert("te-no-replay".into(), json!(true));
}

#[when("I execute the same nondeterministic workflow")]
fn execute_nondeterministic(world: &mut LibWorld) {
    world
        .notes
        .insert("te-run".into(), json!("reads-the-clock"));
    run(world);
}

/// Runs the scenario's `TestEngine` workflow (named by the `te-run` note) on a paused runtime.
pub fn run(world: &mut LibWorld) {
    let which = world.notes["te-run"]
        .as_str()
        .expect("a workflow")
        .to_string();
    let replay = !world.notes.contains_key("te-no-replay");
    let started = std::time::Instant::now();
    let (result, elapsed) = paused(move || {
        runtime().block_on(async move {
            let engine = TestEngine::builder()
                .tasks(Arc::new(Counted::default()))
                .workflow::<WeekApart>()
                .workflow::<ReadsTheClock>()
                .self_replay(replay)
                .build()
                .expect("builds");
            let result = match which.as_str() {
                "week-apart" => match engine.execute(WeekApart::run, ()).await {
                    Ok(handle) => handle.await.map(|n| json!(n)),
                    Err(error) => Err(error),
                },
                _ => match engine.execute(ReadsTheClock::run, ()).await {
                    Ok(handle) => handle.await.map(|n| json!(n)),
                    Err(error) => Err(error),
                },
            };
            (
                result.map_err(|error| format!("{error:?}")),
                engine.clock().elapsed(),
            )
        })
    });
    world
        .notes
        .insert("te-wall".into(), json!(started.elapsed().as_secs_f64()));
    world
        .notes
        .insert("te-elapsed".into(), json!(elapsed.as_secs()));
    match result {
        Ok(output) => world.notes.insert("te-output".into(), output),
        Err(error) => world.notes.insert("te-error".into(), json!(error)),
    };
}

#[then("it completes in under one second of wall-clock time")]
fn under_one_second(world: &mut LibWorld) {
    assert!(
        world.notes.contains_key("te-output"),
        "{:?}",
        world.notes.get("te-error")
    );
    let wall = world.notes["te-wall"].as_f64().expect("timed");
    assert!(wall < 1.0, "took {wall}s");
}

#[then(regex = r#"^"clock\(\)\.elapsed\(\)" is (\d+) days$"#)]
fn elapsed_days(world: &mut LibWorld, days: u64) {
    assert_eq!(world.notes["te-elapsed"], json!(days * 86_400));
}

#[then(regex = r#"^the run fails with "Error::NonDeterminism"$"#)]
fn fails_nondeterminism(world: &mut LibWorld) {
    let error = world
        .notes
        .get("te-error")
        .and_then(Value::as_str)
        .expect("the run failed");
    assert!(error.starts_with("NonDeterminism"), "{error}");
}

#[then("it completes")]
fn it_completes(world: &mut LibWorld) {
    assert!(
        world.notes.contains_key("te-output"),
        "{:?}",
        world.notes.get("te-error")
    );
}

#[given(regex = r#"^the task function "charge"$"#)]
fn task_function(_world: &mut LibWorld) {}

#[when(regex = r#"^I call "charge\(order\)\.await" directly in a test$"#)]
async fn call_directly(world: &mut LibWorld) {
    let output = charge("order-7".to_string()).await.expect("charges");
    world.notes.insert("te-direct".into(), json!(output));
}

#[then("it returns its result without an engine")]
fn returns_without_engine(world: &mut LibWorld) {
    assert_eq!(world.notes["te-direct"], json!("charged order-7"));
    assert!(world.engine.is_none());
}
