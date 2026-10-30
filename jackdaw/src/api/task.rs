//! Native tasks: typed functions the engine can run, with the retry/timeout options a step can carry.
//!
//! Write tasks with the `task` and `tasks` attributes (see the [tasks chapter](crate::guide::tasks)); the types here
//! are what those attributes generate and what a workflow passes to `ctx.call`.
use super::error::{TaskError, error_types};
use futures::FutureExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// A boxed, sendable future. Used where the macros need to name a future type.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Where a task's code lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskTarget {
    /// A native Rust task registered on the engine, by name.
    Native(&'static str),
    /// A catalog entry, `name:version`.
    Catalog {
        /// The entry's name in the catalog.
        name: &'static str,
        /// The entry's version in the catalog.
        version: &'static str,
    },
}

/// The typed marker of a task: its name and its input and output types.
///
/// `ctx.call(marker, input)` takes any `TaskDef`, so the compiler checks the input and gives back the output type.
/// The `task` attribute, a `tasks` impl and a `catalog` trait each produce markers.
pub trait TaskDef: Send + Sync + 'static {
    /// What the task takes.
    type In: Serialize + DeserializeOwned + Send + 'static;
    /// What the task returns.
    type Out: Serialize + DeserializeOwned + Send + 'static;

    /// The name the task is registered and journaled under.
    fn task_name(&self) -> &'static str;

    /// Whether results may be cached by input. Native tasks are side effects, so the default is no.
    fn cacheable(&self) -> bool {
        false
    }

    /// Where the code lives. Native by default.
    fn target(&self) -> TaskTarget {
        TaskTarget::Native(self.task_name())
    }
}

/// The marker of a method in a `#[tasks]` impl (`Shop::charge`). It carries the types and the name, not the code.
pub struct TaskRef<S, I, O> {
    name: &'static str,
    cached: bool,
    _types: PhantomData<fn(S, I) -> O>,
}

impl<S, I, O> TaskRef<S, I, O> {
    /// A marker for the task `name`; `cached` allows results to be reused for equal input.
    #[must_use]
    pub const fn new(name: &'static str, cached: bool) -> Self {
        Self {
            name,
            cached,
            _types: PhantomData,
        }
    }
}

impl<S, I, O> Clone for TaskRef<S, I, O> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S, I, O> Copy for TaskRef<S, I, O> {}

impl<S, I, O> TaskDef for TaskRef<S, I, O>
where
    S: 'static,
    I: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Send + 'static,
{
    type In = I;
    type Out = O;

    fn task_name(&self) -> &'static str {
        self.name
    }

    fn cacheable(&self) -> bool {
        self.cached
    }
}

/// A marker that also carries the code (the unit structs `#[task]` generates for free functions).
pub trait RegisterTask: TaskDef {
    /// The runnable form of the task.
    fn native(self) -> NativeTask;
}

/// The tasks of a `#[tasks]` impl, bound to one instance of the type holding their state.
pub trait TaskSet: Send + Sync + 'static {
    /// One runnable task per method, each sharing `self`.
    fn native_tasks(self: Arc<Self>) -> Vec<NativeTask>;
}

type RunFn = Arc<
    dyn Fn(
            TaskContext,
            serde_json::Value,
        ) -> BoxFuture<'static, Result<serde_json::Value, TaskError>>
        + Send
        + Sync,
>;

/// A task as the engine runs it: untyped values in and out, with the typing done at the edges.
#[derive(Clone)]
pub struct NativeTask {
    pub(crate) name: &'static str,
    pub(crate) cached: bool,
    run: RunFn,
}

impl std::fmt::Debug for NativeTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTask")
            .field("name", &self.name)
            .field("cached", &self.cached)
            .finish()
    }
}

fn invalid_input(error: &serde_json::Error) -> TaskError {
    TaskError::fault(error_types::VALIDATION)
        .status(400)
        .detail(format!("the task's input does not match its type: {error}"))
}

fn unserializable_output(error: &serde_json::Error) -> TaskError {
    TaskError::non_retryable(format!(
        "the task's output could not be serialized: {error}"
    ))
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the task panicked".to_string())
}

impl NativeTask {
    /// An async task. Input is decoded from JSON and output encoded to JSON; a panic becomes a non-retryable failure.
    pub fn new<I, O, F, Fut>(name: &'static str, cached: bool, f: F) -> Self
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(TaskContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, TaskError>> + Send + 'static,
    {
        let f = Arc::new(f);
        Self {
            name,
            cached,
            run: Arc::new(move |ctx, input| {
                let f = Arc::clone(&f);
                Box::pin(async move {
                    let input: I = serde_json::from_value(input).map_err(|e| invalid_input(&e))?;
                    let output = AssertUnwindSafe(async move { f(ctx, input).await })
                        .catch_unwind()
                        .await
                        .map_err(|payload| {
                            TaskError::non_retryable(panic_message(payload.as_ref()))
                        })??;
                    serde_json::to_value(output).map_err(|e| unserializable_output(&e))
                })
            }),
        }
    }

    /// A synchronous task, run on tokio's blocking pool so it cannot stall the runtime.
    pub fn blocking<I, O, F>(name: &'static str, cached: bool, f: F) -> Self
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(TaskContext, I) -> Result<O, TaskError> + Send + Sync + 'static,
    {
        let f = Arc::new(f);
        Self {
            name,
            cached,
            run: Arc::new(move |ctx, input| {
                let f = Arc::clone(&f);
                Box::pin(async move {
                    let input: I = serde_json::from_value(input).map_err(|e| invalid_input(&e))?;
                    let output = super::observe::spawn_blocking(move || f(ctx, input))
                        .await
                        .map_err(|error| {
                            if error.is_panic() {
                                TaskError::non_retryable(panic_message(error.into_panic().as_ref()))
                            } else {
                                TaskError::non_retryable("the blocking task was cancelled")
                            }
                        })??;
                    serde_json::to_value(output).map_err(|e| unserializable_output(&e))
                })
            }),
        }
    }

    pub(crate) fn invoke(
        &self,
        ctx: TaskContext,
        input: serde_json::Value,
    ) -> BoxFuture<'static, Result<serde_json::Value, TaskError>> {
        (self.run)(ctx, input)
    }
}

/// What a task can learn about the call it is serving.
#[derive(Debug, Clone)]
pub struct TaskContext {
    instance_id: String,
    task_name: String,
    attempt: u32,
    idempotency_key: String,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
    heartbeat: Option<std::sync::Arc<Heartbeat>>,
}

/// The heartbeats of one step: the latest progress (kept across attempts) and when it last beat.
#[derive(Debug)]
pub(crate) struct Heartbeat {
    pub(crate) beats: tokio::sync::watch::Sender<u64>,
    pub(crate) progress: std::sync::Mutex<Option<serde_json::Value>>,
}

impl Heartbeat {
    pub(crate) fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            beats: tokio::sync::watch::channel(0).0,
            progress: std::sync::Mutex::new(None),
        })
    }
}

impl TaskContext {
    pub(crate) fn new(
        instance_id: &str,
        task_name: &str,
        attempt: u32,
        idempotency_key: String,
    ) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            task_name: task_name.to_string(),
            attempt,
            idempotency_key,
            cancel: None,
            heartbeat: None,
        }
    }

    pub(crate) fn with_heartbeat(mut self, heartbeat: std::sync::Arc<Heartbeat>) -> Self {
        self.heartbeat = Some(heartbeat);
        self
    }

    /// Reports that the task is alive, with how far it got. A step with a
    /// [`heartbeat_timeout`](crate::api::Step::heartbeat_timeout) fails an attempt that stops heartbeating; the next
    /// attempt can read the progress with [`last_heartbeat`](Self::last_heartbeat) and continue from there.
    pub fn heartbeat(&self, progress: impl serde::Serialize) {
        if let Some(heartbeat) = &self.heartbeat {
            if let Ok(value) = serde_json::to_value(progress) {
                *heartbeat
                    .progress
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
            }
            heartbeat.beats.send_modify(|count| *count += 1);
        }
    }

    /// The progress of the latest heartbeat of this step, from this attempt or an earlier one.
    #[must_use]
    pub fn last_heartbeat<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        let heartbeat = self.heartbeat.as_ref()?;
        let progress = heartbeat
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        serde_json::from_value(progress).ok()
    }

    pub(crate) fn with_cancel(mut self, cancel: tokio::sync::watch::Receiver<bool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Whether the workflow instance running this task has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|cancel| *cancel.borrow())
    }

    /// Resolves when the workflow instance running this task is cancelled. A long-running task can race its work
    /// against it to stop early; it never resolves for a task run outside a workflow.
    pub async fn cancelled(&self) {
        let Some(mut cancel) = self.cancel.clone() else {
            return std::future::pending().await;
        };
        while !*cancel.borrow_and_update() {
            if cancel.changed().await.is_err() {
                return std::future::pending().await;
            }
        }
    }

    /// The workflow instance running this task.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// The name the task was registered under.
    #[must_use]
    pub fn task_name(&self) -> &str {
        &self.task_name
    }

    /// The attempt number, starting at 1.
    #[must_use]
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Stable across retries and replay of the same step: use it to make external side effects idempotent.
    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
}

/// How a failing step is retried.
///
/// Only failures marked retryable are retried; see [`TaskError`].
///
/// ```
/// use jackdaw::prelude::*;
///
/// let options = TaskOptions::new()
///     .retry(Retry::exponential(5))
///     .timeout(10.secs());
/// # let _ = options;
/// ```
#[derive(Debug, Clone)]
pub struct Retry {
    backoff: Backoff,
    max_attempts: u32,
}

#[derive(Debug, Clone)]
enum Backoff {
    Constant(Duration),
    Exponential { initial: Duration, cap: Duration },
}

impl Retry {
    /// Waits `delay` between attempts. Three attempts unless changed with [`Retry::max_attempts`].
    #[must_use]
    pub fn constant(delay: Duration) -> Self {
        Self {
            backoff: Backoff::Constant(delay),
            max_attempts: 3,
        }
    }

    /// Doubles the wait after each failure (100ms, 200ms, ...), up to `max_attempts` attempts in total.
    #[must_use]
    pub fn exponential(max_attempts: u32) -> Self {
        Self {
            backoff: Backoff::Exponential {
                initial: Duration::from_millis(100),
                cap: Duration::from_secs(30),
            },
            max_attempts,
        }
    }

    /// The total number of attempts, including the first.
    #[must_use]
    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
    }

    pub(crate) fn attempts(&self) -> u32 {
        self.max_attempts.max(1)
    }

    /// The wait after `failed_attempts` consecutive failures.
    pub(crate) fn delay_after(&self, failed_attempts: u32) -> Duration {
        match &self.backoff {
            Backoff::Constant(delay) => *delay,
            Backoff::Exponential { initial, cap } => {
                let factor = 2u32.saturating_pow(failed_attempts.saturating_sub(1));
                initial.saturating_mul(factor).min(*cap)
            }
        }
    }
}

/// Retry and timeout for a step.
#[derive(Debug, Clone, Default)]
pub struct TaskOptions {
    pub(crate) retry: Option<Retry>,
    pub(crate) timeout: Option<Duration>,
    /// Overrides the task's own `#[task(cached)]` setting for one call.
    pub(crate) cache: Option<bool>,
    /// Fails an attempt that goes this long without a heartbeat.
    pub(crate) heartbeat_timeout: Option<Duration>,
}

impl TaskOptions {
    /// No retry and no timeout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retry failed attempts as `retry` says.
    #[must_use]
    pub fn retry(mut self, retry: Retry) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Give up on an attempt that takes longer than `timeout`.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Reuse a cached result for equal input (and cache this one), whether or not the task is `#[task(cached)]`.
    #[must_use]
    pub fn cached(mut self) -> Self {
        self.cache = Some(true);
        self
    }

    /// Run the task even if it is `#[task(cached)]` and an equal input was cached.
    #[must_use]
    pub fn uncached(mut self) -> Self {
        self.cache = Some(false);
        self
    }

    /// Fail an attempt that goes `limit` without calling [`TaskContext::heartbeat`]; the retry policy decides whether
    /// another attempt follows.
    #[must_use]
    pub fn heartbeat_timeout(mut self, limit: Duration) -> Self {
        self.heartbeat_timeout = Some(limit);
        self
    }
}
