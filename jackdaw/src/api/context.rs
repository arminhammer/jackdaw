//! `WorkflowContext`: what a workflow's `#[run]` uses to do things, and the futures those things return.
//!
//! Every method that creates a step (`call`, `run`, `sleep`, `wait_for_event`, ...) fixes the step's identity at the
//! moment it is called, not when the returned future is first polled. That is what makes `futures::try_join!` and
//! `join_all` safe to use: replay does not depend on poll order.
use super::error::{Error, Fault, StepError, WorkflowResult};
use super::runtime::{InstanceCore, StepExec, StepSpec};
use super::task::{BoxFuture, Retry, TaskDef, TaskOptions};
use super::workflow::{RunTarget, Runnable, Workflow, WorkflowDef};
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::borrow::Borrow;
use std::future::Future;
use std::marker::PhantomData;
use std::panic::Location;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a workflow's `#[run]` method gets. Its methods take `&self`, so the futures they return can be combined.
pub struct WorkflowContext<W> {
    core: Arc<InstanceCore>,
    state: Arc<Mutex<W>>,
}

fn invalid_output(step: &str, error: &serde_json::Error) -> StepError {
    StepError::Fault(Fault::validation(format!(
        "the output of {step} does not match its type: {error}"
    )))
}

impl<W: WorkflowDef> WorkflowContext<W> {
    /// Ends this run and starts a fresh run of the same instance with `input`: same instance id and handles, a new
    /// journal and new state. Use it to keep the history of a long-lived workflow (a poller, a subscription) small.
    ///
    /// Return the result from `#[run]`: `return ctx.continue_as_new(next);`.
    ///
    /// # Errors
    /// Always: the "error" is how the request reaches the engine.
    pub fn continue_as_new<T>(&self, input: W::In) -> WorkflowResult<T> {
        match serde_json::to_value(&input) {
            Ok(value) => Err(StepError::ContinuedAsNew(Box::new(value))),
            Err(error) => Err(invalid_input(&error)),
        }
    }
}

/// How a step is named in errors: "task `charge`", "event `paid`", ...
fn describe(kind: &str, name: &str) -> String {
    let noun = match kind {
        "call" => "task",
        "run" => "child workflow",
        other => other,
    };
    format!("{noun} `{name}`")
}

impl<W: Send + 'static> WorkflowContext<W> {
    pub(crate) fn new(core: Arc<InstanceCore>, state: Arc<Mutex<W>>) -> Self {
        Self { core, state }
    }

    /// The id of the running instance.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.core.id
    }

    /// Reads the workflow's state (the fields of the `#[workflow]` struct).
    pub fn state<R>(&self, read: impl FnOnce(&W) -> R) -> R {
        read(&lock(&self.state))
    }

    /// Changes the workflow's fields. Like everything in `#[run]`, the change must follow from the steps' results so a
    /// replay makes the same change; queries see it immediately and `wait_condition`s are re-checked.
    pub fn state_mut<R>(&self, change: impl FnOnce(&mut W) -> R) -> R {
        let result = change(&mut lock(&self.state));
        self.core.state_changed.notify_waiters();
        result
    }

    /// Runs a task. Chain `.retry(..)` and `.timeout(..)` before awaiting.
    #[track_caller]
    pub fn call<T: TaskDef>(&self, task: T, input: impl Borrow<T::In>) -> Step<T::Out> {
        let location = Location::caller();
        match serde_json::to_value(input.borrow()) {
            Ok(value) => {
                let name = task.task_name().to_string();
                let spec = self.core.spec(
                    "call",
                    name,
                    value,
                    location,
                    StepExec::Task {
                        target: task.target(),
                        options: TaskOptions::new(),
                    },
                );
                Step::new(Arc::clone(&self.core), spec)
            }
            Err(error) => Step::failed(Arc::clone(&self.core), invalid_input(&error)),
        }
    }

    /// Runs another workflow and returns its output: a Rust workflow (`Child::run`), a YAML workflow
    /// (`&Workflow<In, Out>`), or a catalog workflow.
    #[track_caller]
    pub fn run<R: Runnable>(&self, child: R, input: impl Borrow<R::In>) -> Step<R::Out> {
        let location = Location::caller();
        match serde_json::to_value(input.borrow()) {
            Ok(value) => {
                let target = child.target();
                let name = match &target {
                    RunTarget::Native(info) => info.id.name.clone(),
                    RunTarget::Definition(definition) => definition.document.name.clone(),
                    RunTarget::Catalog { name, .. } => (*name).to_string(),
                };
                let spec = self
                    .core
                    .spec("run", name, value, location, StepExec::Child(target));
                Step::new(Arc::clone(&self.core), spec)
            }
            Err(error) => Step::failed(Arc::clone(&self.core), invalid_input(&error)),
        }
    }

    /// Starts another workflow and returns its instance id without waiting for it to finish. The started workflow is
    /// independent: it keeps running after this one ends, and [`Engine::handle`](crate::api::Engine::handle) finds it
    /// by the returned id. Starting is a recorded step, so a replay does not start it again.
    #[track_caller]
    pub fn start<R: Runnable>(&self, workflow: R, input: impl Borrow<R::In>) -> Step<String> {
        let location = Location::caller();
        match serde_json::to_value(input.borrow()) {
            Ok(value) => {
                let target = workflow.target();
                let name = match &target {
                    RunTarget::Native(info) => info.id.name.clone(),
                    RunTarget::Definition(definition) => definition.document.name.clone(),
                    RunTarget::Catalog { name, .. } => (*name).to_string(),
                };
                let spec = self
                    .core
                    .spec("start", name, value, location, StepExec::Start(target));
                Step::new(Arc::clone(&self.core), spec)
            }
            Err(error) => Step::failed(Arc::clone(&self.core), invalid_input(&error)),
        }
    }

    /// Runs a YAML workflow bound to Rust types as a child.
    #[track_caller]
    pub fn run_definition<I, O>(&self, workflow: &Workflow<I, O>, input: &I) -> Step<O>
    where
        I: Serialize + Send + 'static,
        O: DeserializeOwned + Send + 'static,
    {
        self.run(workflow, input)
    }

    /// Runs `task` once for each item, in order, and collects the outputs. Sequential unless `.concurrency(n)` is set.
    #[track_caller]
    pub fn for_each<T: TaskDef>(&self, items: &[T::In], task: T) -> ForEach<T::Out> {
        let location = Location::caller();
        let mut specs = Vec::with_capacity(items.len());
        for item in items {
            match serde_json::to_value(item) {
                Ok(value) => specs.push(self.core.spec(
                    "call",
                    task.task_name().to_string(),
                    value,
                    location,
                    StepExec::Task {
                        target: task.target(),
                        options: TaskOptions::new(),
                    },
                )),
                Err(error) => {
                    return ForEach::failed(Arc::clone(&self.core), invalid_input(&error));
                }
            }
        }
        ForEach::new(Arc::clone(&self.core), specs)
    }

    /// A durable timer.
    #[track_caller]
    #[must_use]
    pub fn sleep(&self, duration: Duration) -> Step<()> {
        let spec = self.core.spec(
            "sleep",
            "sleep".to_string(),
            json!(duration.as_millis().to_string()),
            Location::caller(),
            StepExec::Sleep(duration),
        );
        Step::new(Arc::clone(&self.core), spec)
    }

    /// Runs a command on the engine's machine and returns what it printed. A non-zero exit fails the step with a
    /// fault titled `exit code N`. See [`Shell`](super::Shell).
    #[track_caller]
    #[must_use]
    pub fn run_shell(&self, shell: super::Shell) -> Step<super::ProcessOutput> {
        self.call(super::helpers::ShellTask, shell)
    }

    /// Runs a Python or JavaScript script with its interpreter. See [`Script`](super::Script).
    #[track_caller]
    #[must_use]
    pub fn run_script(&self, script: super::Script) -> Step<super::ProcessOutput> {
        self.call(super::helpers::ScriptTask, script)
    }

    /// Runs a container with Docker and returns what it printed. See [`Container`](super::Container).
    #[track_caller]
    #[must_use]
    pub fn run_container(&self, container: super::Container) -> Step<super::ProcessOutput> {
        self.call(super::helpers::ContainerTask, container)
    }

    /// Sends an HTTP request. Statuses 408, 429 and 5xx are retryable faults, so `.retry(..)` on the step retries
    /// them. See [`Request`](super::Request).
    #[track_caller]
    #[must_use]
    pub fn http(&self, request: super::Request) -> Step<super::Response> {
        self.call(super::helpers::HttpTask, request)
    }

    /// Publishes `event` on this run's event stream and in its history. It is a step: a replay does not publish it
    /// again. If the engine stops between publishing and recording the step, the event is published again on resume
    /// with the same id, so consumers can drop the duplicate.
    #[track_caller]
    #[must_use]
    pub fn emit(&self, event: super::event::CloudEvent) -> Step<()> {
        let location = Location::caller();
        match serde_json::to_value(&event) {
            Ok(value) => {
                let spec =
                    self.core
                        .spec("emit", event.event_type, value, location, StepExec::Emit);
                Step::new(Arc::clone(&self.core), spec)
            }
            Err(error) => Step::failed(Arc::clone(&self.core), invalid_input(&error)),
        }
    }

    /// Waits for an event sent with `ExecutionHandle::send_event`.
    #[track_caller]
    #[must_use]
    pub fn wait_for_event<T: DeserializeOwned>(&self, name: &str) -> Step<T> {
        let spec = self.core.spec(
            "event",
            name.to_string(),
            Value::Null,
            Location::caller(),
            StepExec::Event,
        );
        Step::new(Arc::clone(&self.core), spec)
    }

    /// Waits until `condition` is true of the workflow's state (it is re-checked after every signal).
    ///
    /// The returned [`Condition`] can be raced with [`select!`](crate::select), for example against a timer.
    #[track_caller]
    #[must_use]
    pub fn wait_condition<F>(&self, condition: F) -> Condition
    where
        F: Fn(&W) -> bool + Send + Sync + 'static,
    {
        let core = Arc::clone(&self.core);
        let state = Arc::clone(&self.state);
        let location = Location::caller();
        let waiting = Arc::clone(&core);
        Condition {
            core,
            waiting: Box::pin(async move {
                let check = move || condition(&lock(&state));
                waiting.wait_until(&check, location).await
            }),
        }
    }

    /// The current time, recorded: a replay sees the value the first run saw.
    #[track_caller]
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        let value = self
            .core
            .recorded_value("now", Location::caller(), || json!(self.core.clock.now()));
        serde_json::from_value(value).unwrap_or_else(|_| self.core.clock.now())
    }

    /// A number in `[0, 1)`, recorded.
    #[track_caller]
    #[must_use]
    pub fn random(&self) -> f64 {
        // The journal keeps the bit pattern, not the float: JSON text does not always give back the same f64, and a
        // replayed value that differs in the last digit would change the hash of whatever step it is passed to.
        let value = self.core.recorded_value("random", Location::caller(), || {
            // 53 random bits make an exactly representable fraction in [0, 1).
            let bits = u64::try_from(uuid::Uuid::new_v4().as_u128() >> 75).unwrap_or(0);
            let fraction = f64::from(u32::try_from(bits >> 21).unwrap_or(0)) / 4_294_967_296.0
                + f64::from(u32::try_from(bits & 0x1f_ffff).unwrap_or(0))
                    / 4_294_967_296.0
                    / 2_097_152.0;
            json!(fraction.to_bits())
        });
        value.as_u64().map_or(0.0, f64::from_bits)
    }

    /// A random UUID, recorded.
    #[track_caller]
    #[must_use]
    pub fn uuid(&self) -> uuid::Uuid {
        let value = self.core.recorded_value("uuid", Location::caller(), || {
            json!(uuid::Uuid::new_v4().to_string())
        });
        value
            .as_str()
            .and_then(|text| uuid::Uuid::parse_str(text).ok())
            .unwrap_or_else(uuid::Uuid::nil)
    }

    /// True for executions that started with this code, false when replaying a history that started before the
    /// change. Use it to change a running workflow in place.
    #[must_use]
    pub fn patched(&self, id: &str) -> bool {
        self.core.patched(id)
    }
}

fn invalid_input(error: &serde_json::Error) -> StepError {
    StepError::Fault(Fault::validation(format!(
        "a step's input could not be serialized: {error}"
    )))
}

// ---------------------------------------------------------------------------------------------------------------
// A single step
// ---------------------------------------------------------------------------------------------------------------

/// A step in progress. It is a future: `.await` it, combine it with `futures::join!`, or race it with `select!`.
pub struct Step<Out> {
    core: Arc<InstanceCore>,
    spec: Option<StepSpec>,
    options: TaskOptions,
    failed: Option<StepError>,
    running: Option<BoxFuture<'static, Result<Value, StepError>>>,
    label: String,
    shielded: bool,
    _out: PhantomData<fn() -> Out>,
}

impl<Out> Step<Out> {
    fn new(core: Arc<InstanceCore>, spec: StepSpec) -> Self {
        Self {
            core,
            label: describe(spec.kind, &spec.name),
            shielded: false,
            spec: Some(spec),
            options: TaskOptions::new(),
            failed: None,
            running: None,
            _out: PhantomData,
        }
    }

    fn failed(core: Arc<InstanceCore>, error: StepError) -> Self {
        Self {
            core,
            spec: None,
            options: TaskOptions::new(),
            failed: Some(error),
            running: None,
            label: String::new(),
            shielded: false,
            _out: PhantomData,
        }
    }

    /// Retries failures that the task marks retryable, according to `retry`.
    #[must_use]
    pub fn retry(mut self, retry: Retry) -> Self {
        self.options = self.options.retry(retry);
        self
    }

    /// Fails the step with a timeout if it takes longer than `limit`.
    #[must_use]
    pub fn timeout(mut self, limit: Duration) -> Self {
        self.options = self.options.timeout(limit);
        self
    }

    /// Reuses a cached result for equal input, and caches this one, even if the task is not `#[task(cached)]`.
    #[must_use]
    pub fn cached(mut self) -> Self {
        self.options = self.options.cached();
        self
    }

    /// Runs the task even if it is `#[task(cached)]` and an equal input was cached.
    #[must_use]
    pub fn uncached(mut self) -> Self {
        self.options = self.options.uncached();
        self
    }

    /// Fails an attempt of the task that goes `limit` without a heartbeat (see [`TaskContext::heartbeat`]).
    ///
    /// [`TaskContext::heartbeat`]: crate::api::TaskContext::heartbeat
    #[must_use]
    pub fn heartbeat_timeout(mut self, limit: Duration) -> Self {
        self.options = self.options.heartbeat_timeout(limit);
        self
    }

    /// Runs the step even if the instance is cancelled, for clean-up such as releasing a reservation. Other steps
    /// fail with [`StepError::Cancelled`] once the instance is cancelled.
    #[must_use]
    pub fn shielded(mut self) -> Self {
        self.shielded = true;
        self
    }
}

impl<Out: DeserializeOwned> Future for Step<Out> {
    type Output = Result<Out, StepError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(error) = this.failed.take() {
            return Poll::Ready(Err(error));
        }
        if this.running.is_none() {
            let Some(mut spec) = this.spec.take() else {
                return Poll::Ready(Err(StepError::Engine(Error::Engine(
                    "a step was polled again after it finished".to_string(),
                ))));
            };
            if let StepExec::Task { options, .. } = &mut spec.exec {
                *options = this.options.clone();
            }
            spec.timeout = this.options.timeout;
            spec.shielded = this.shielded;
            let core = Arc::clone(&this.core);
            this.running = Some(Box::pin(async move { core.run_step(spec).await }));
        }
        match this.running.as_mut() {
            Some(running) => running.as_mut().poll(cx).map(|result| {
                result.and_then(|value| {
                    serde_json::from_value(value).map_err(|e| invalid_output(&this.label, &e))
                })
            }),
            None => Poll::Pending,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// for_each
// ---------------------------------------------------------------------------------------------------------------

/// `ctx.for_each(..)`: one task call per item, with the outputs collected in order.
pub struct ForEach<Out> {
    core: Arc<InstanceCore>,
    specs: Vec<StepSpec>,
    concurrency: usize,
    failed: Option<StepError>,
    running: Option<BoxFuture<'static, Result<Vec<Value>, StepError>>>,
    _out: PhantomData<fn() -> Out>,
}

impl<Out> ForEach<Out> {
    fn new(core: Arc<InstanceCore>, specs: Vec<StepSpec>) -> Self {
        Self {
            core,
            specs,
            concurrency: 1,
            failed: None,
            running: None,
            _out: PhantomData,
        }
    }

    fn failed(core: Arc<InstanceCore>, error: StepError) -> Self {
        let mut each = Self::new(core, Vec::new());
        each.failed = Some(error);
        each
    }

    /// How many items run at once (default 1: the spec's `for` is sequential).
    #[must_use]
    pub fn concurrency(mut self, limit: usize) -> Self {
        self.concurrency = limit.max(1);
        self
    }
}

impl<Out: DeserializeOwned> Future for ForEach<Out> {
    type Output = Result<Vec<Out>, StepError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(error) = this.failed.take() {
            return Poll::Ready(Err(error));
        }
        if this.running.is_none() {
            let core = Arc::clone(&this.core);
            let specs = std::mem::take(&mut this.specs);
            let limit = this.concurrency;
            this.running = Some(Box::pin(async move {
                futures::stream::iter(specs)
                    .map(move |spec| {
                        let core = Arc::clone(&core);
                        async move { core.run_step(spec).await }
                    })
                    .buffered(limit)
                    .try_collect::<Vec<Value>>()
                    .await
            }));
        }
        match this.running.as_mut() {
            Some(running) => running.as_mut().poll(cx).map(|result| {
                result.and_then(|values| {
                    values
                        .into_iter()
                        .map(|value| {
                            serde_json::from_value(value)
                                .map_err(|e| invalid_output("a for_each item", &e))
                        })
                        .collect()
                })
            }),
            None => Poll::Pending,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// select!
// ---------------------------------------------------------------------------------------------------------------

/// The winner of a race.
pub enum Either<A, B> {
    /// The first step finished first.
    Left(A),
    /// The second step finished first.
    Right(B),
}

/// Opaque handle to the instance a step belongs to (for [`Branch`]).
#[doc(hidden)]
pub struct CoreRef(Arc<InstanceCore>);

/// A future that belongs to a workflow instance, so a race can journal which branch won.
pub trait Branch {
    #[doc(hidden)]
    fn core(&self) -> CoreRef;
}

impl<Out> Branch for Step<Out> {
    fn core(&self) -> CoreRef {
        CoreRef(Arc::clone(&self.core))
    }
}

/// `ctx.wait_condition(..)` in progress: a future that resolves once the condition holds. It can be raced with
/// [`select!`](crate::select).
pub struct Condition {
    core: Arc<InstanceCore>,
    waiting: BoxFuture<'static, Result<(), StepError>>,
}

impl Future for Condition {
    type Output = Result<(), StepError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().waiting.as_mut().poll(cx)
    }
}

impl Branch for Condition {
    fn core(&self) -> CoreRef {
        CoreRef(Arc::clone(&self.core))
    }
}

/// Runs two steps against each other and returns the first to finish, dropping the other. The winner is journaled, so a
/// replay picks the same branch without starting the loser. Use through [`select!`](crate::select).
#[track_caller]
pub fn race<A, B, X, Y>(first: A, second: B) -> impl Future<Output = Either<X, Y>>
where
    A: Future<Output = X> + Branch + Unpin,
    B: Future<Output = Y> + Unpin,
{
    let CoreRef(core) = first.core();
    let spec = core.spec(
        "race",
        "race".to_string(),
        Value::Null,
        Location::caller(),
        StepExec::Event,
    );
    async move {
        match core.recorded_winner(&spec) {
            Some(0) => Either::Left(first.await),
            Some(_) => Either::Right(second.await),
            None => match futures::future::select(first, second).await {
                futures::future::Either::Left((value, _)) => {
                    core.record_winner(&spec, 0);
                    Either::Left(value)
                }
                futures::future::Either::Right((value, _)) => {
                    core.record_winner(&spec, 1);
                    Either::Right(value)
                }
            },
        }
    }
}

/// Waits for the first of two steps to finish: `jackdaw::select! { a = step_a => .., b = step_b => .. }`.
///
/// Unlike `tokio::select!`, the winner is recorded, so replaying the workflow takes the same branch.
#[macro_export]
macro_rules! select {
    ($first_pat:pat = $first:expr => $first_body:expr, $second_pat:pat = $second:expr => $second_body:expr $(,)?) => {
        match $crate::api::race($first, $second).await {
            $crate::api::Either::Left($first_pat) => $first_body,
            $crate::api::Either::Right($second_pat) => $second_body,
        }
    };
}
