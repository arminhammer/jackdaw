//! Native tasks: typed input and output, errors that become spec faults, retries, timeouts, panics, caching.
//!
//! `engine.run_task(marker, input, options)` runs a single task through the same machinery a workflow step
//! uses (retries, timeouts, events, persistence). The tests use `TestEngine`, which runs on tokio's paused
//! clock: time only moves when everything is waiting, so these tests never really sleep.
use crate::support::*;
use jackdaw::testing::TestEngine;

/// What a task can learn about the call it is serving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallInfo {
    pub instance_id: String,
    pub task_name: String,
    pub attempt: u32,
    pub idempotency_key: String,
}

pub struct Probe {
    pub calls: Counter,
    /// How many calls to `flaky` fail before one succeeds.
    pub fail_first: usize,
}

impl Probe {
    pub fn new(fail_first: usize) -> Arc<Self> {
        Arc::new(Self {
            calls: Counter::default(),
            fail_first,
        })
    }
}

#[tasks]
impl Probe {
    #[task]
    pub async fn flaky(self: Arc<Self>, input: Name) -> Result<Greeting, TaskError> {
        if self.calls.hit() <= self.fail_first {
            return Err(TaskError::retryable("temporary outage"));
        }
        Ok(Greeting::hello(&input.name))
    }

    #[task]
    pub async fn invalid(self: Arc<Self>, _input: Name) -> Result<Greeting, TaskError> {
        self.calls.hit();
        Err(TaskError::non_retryable("bad input"))
    }

    #[task]
    pub async fn bad_gateway(self: Arc<Self>, _input: Name) -> Result<Greeting, TaskError> {
        self.calls.hit();
        Err(TaskError::fault(error_types::COMMUNICATION)
            .status(502)
            .detail("gateway down"))
    }

    #[task]
    pub async fn slow(self: Arc<Self>, input: Name) -> Result<Greeting, TaskError> {
        tokio::time::sleep(1.hours()).await;
        Ok(Greeting::hello(&input.name))
    }

    #[task]
    pub async fn explode(self: Arc<Self>, _input: Name) -> Result<Greeting, TaskError> {
        panic!("boom");
    }

    #[task]
    pub async fn uncached(self: Arc<Self>, input: Name) -> Result<Greeting, TaskError> {
        self.calls.hit();
        Ok(Greeting::hello(&input.name))
    }

    #[task(cached)]
    pub async fn cached(self: Arc<Self>, input: Name) -> Result<Greeting, TaskError> {
        self.calls.hit();
        Ok(Greeting::hello(&input.name))
    }

    #[task]
    pub async fn who_am_i(
        self: Arc<Self>,
        ctx: TaskContext,
        _input: Name,
    ) -> Result<CallInfo, TaskError> {
        if self.calls.hit() == 1 {
            return Err(TaskError::retryable("first attempt fails"));
        }
        Ok(CallInfo {
            instance_id: ctx.instance_id().to_string(),
            task_name: ctx.task_name().to_string(),
            attempt: ctx.attempt(),
            idempotency_key: ctx.idempotency_key().to_string(),
        })
    }
}

/// A synchronous, CPU-bound function. `blocking` runs it off the async runtime's threads.
#[task(blocking)]
pub fn checksum(input: Name) -> Result<u64, TaskError> {
    Ok(input.name.bytes().map(u64::from).sum())
}

fn engine(probe: &Arc<Probe>) -> TestEngine {
    TestEngine::builder()
        .task(Greet)
        .tasks(probe.clone())
        .task(Checksum)
        .build()
        .unwrap()
}

fn three_attempts() -> TaskOptions {
    TaskOptions::new().retry(Retry::constant(10.millis()).max_attempts(3))
}

#[tokio::test(start_paused = true)]
async fn a_task_returns_its_typed_output() -> TestResult {
    let engine = TestEngine::builder().task(Greet).build()?;

    let greeting: Greeting = engine
        .run_task(Greet, Name::new("Ada"), TaskOptions::new())
        .await?;

    assert_eq!(greeting, Greeting::hello("Ada"));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_blocking_task_runs_and_returns_its_output() -> TestResult {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    let sum: u64 = engine
        .run_task(Checksum, Name::new("ab"), TaskOptions::new())
        .await?;

    assert_eq!(sum, u64::from(b'a') + u64::from(b'b'));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_non_retryable_error_faults_without_retrying() {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    let err = engine
        .run_task(Probe::invalid, Name::new("Ada"), three_attempts())
        .await
        .unwrap_err();

    let Error::Faulted(fault) = err else {
        panic!("expected a fault, got {err:?}");
    };
    assert_eq!(fault.kind, error_types::RUNTIME);
    assert!(fault.detail.unwrap().contains("bad input"));
    assert_eq!(
        probe.calls.get(),
        1,
        "a non-retryable error is never retried"
    );
}

#[tokio::test(start_paused = true)]
async fn a_task_error_can_carry_the_spec_fault_fields() {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    let err = engine
        .run_task(Probe::bad_gateway, Name::new("Ada"), TaskOptions::new())
        .await
        .unwrap_err();

    let Error::Faulted(fault) = err else {
        panic!("expected a fault, got {err:?}");
    };
    assert_eq!(fault.kind, error_types::COMMUNICATION);
    assert_eq!(fault.status, Some(502));
    assert_eq!(fault.detail.as_deref(), Some("gateway down"));
}

#[tokio::test(start_paused = true)]
async fn retryable_errors_are_retried_per_the_policy() -> TestResult {
    let probe = Probe::new(2);
    let engine = engine(&probe);

    let greeting = engine
        .run_task(Probe::flaky, Name::new("Ada"), three_attempts())
        .await?;

    assert_eq!(greeting, Greeting::hello("Ada"));
    assert_eq!(probe.calls.get(), 3, "two failures, then a success");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn retries_stop_at_the_attempt_limit() {
    let probe = Probe::new(10);
    let engine = engine(&probe);

    let err = engine
        .run_task(Probe::flaky, Name::new("Ada"), three_attempts())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Faulted(_)), "got {err:?}");
    assert_eq!(probe.calls.get(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_task_timeout_is_reported_as_a_timeout() {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    let err = engine
        .run_task(
            Probe::slow,
            Name::new("Ada"),
            TaskOptions::new().timeout(10.millis()),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Timeout { .. }), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_panicking_task_is_a_runtime_fault_and_does_not_take_the_engine_down() -> TestResult {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    let err = engine
        .run_task(Probe::explode, Name::new("Ada"), TaskOptions::new())
        .await
        .unwrap_err();

    let Error::Faulted(fault) = err else {
        panic!("expected a fault, got {err:?}");
    };
    assert_eq!(fault.kind, error_types::RUNTIME);
    assert!(fault.detail.unwrap().contains("boom"));

    // The engine is still usable afterwards.
    let greeting = engine
        .run_task(Greet, Name::new("Ada"), TaskOptions::new())
        .await?;
    assert_eq!(greeting, Greeting::hello("Ada"));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn tasks_are_not_cached_by_default() -> TestResult {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    engine
        .run_task(Probe::uncached, Name::new("Ada"), TaskOptions::new())
        .await?;
    engine
        .run_task(Probe::uncached, Name::new("Ada"), TaskOptions::new())
        .await?;

    // A task is a side effect. Running it twice with the same input must run it twice.
    assert_eq!(probe.calls.get(), 2);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_cached_task_runs_once_per_distinct_input() -> TestResult {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    for name in ["Ada", "Ada", "Grace"] {
        engine
            .run_task(Probe::cached, Name::new(name), TaskOptions::new())
            .await?;
    }

    assert_eq!(probe.calls.get(), 2, "the second Ada comes from the cache");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_task_can_see_its_instance_attempt_and_idempotency_key() -> TestResult {
    let probe = Probe::new(0);
    let engine = engine(&probe);

    let info: CallInfo = engine
        .run_task(Probe::who_am_i, Name::new("Ada"), three_attempts())
        .await?;

    assert_eq!(info.task_name, "who_am_i");
    assert_eq!(
        info.attempt, 2,
        "the first attempt failed, this is the second"
    );
    assert!(!info.instance_id.is_empty());
    assert!(!info.idempotency_key.is_empty());
    Ok(())
}

const CALLS_GREET_YAML: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: calls-greet, version: '1.0.0' }
do:
  - hello:
      call: greet
      with:
        name: '${ .name }'
"#;

#[tokio::test(start_paused = true)]
async fn yaml_can_call_a_registered_task_by_name() -> TestResult {
    let engine = TestEngine::builder().task(Greet).build()?;
    let workflow = Workflow::<Name, Greeting>::from_yaml(CALLS_GREET_YAML)?;

    let greeting = engine.execute(&workflow, Name::new("Ada")).await?.await?;

    assert_eq!(greeting, Greeting::hello("Ada"));
    Ok(())
}

const CALLS_UNKNOWN_YAML: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: calls-unknown, version: '1.0.0' }
do:
  - first:
      call: uncached
      with: { name: Ada }
  - second:
      call: does-not-exist
"#;

#[tokio::test(start_paused = true)]
async fn an_unknown_call_is_rejected_before_anything_runs() {
    let probe = Probe::new(0);
    let engine = engine(&probe);
    let workflow = Workflow::<Name, Greeting>::from_yaml(CALLS_UNKNOWN_YAML).unwrap();

    let err = engine
        .execute(&workflow, Name::new("Ada"))
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(err.to_string().contains("does-not-exist"));
    assert_eq!(
        probe.calls.get(),
        0,
        "validation happens before the first task runs"
    );
}
