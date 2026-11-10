//! Tracing spans and metrics.
//!
//! Every instance runs in a `workflow` span and every live step in a `step` span inside it, so logs from task code
//! carry the workflow, instance and step they belong to. Replayed steps are not run again and get no span, so nothing
//! is logged twice. With the `metrics` feature the engine also reports counters and histograms through the `metrics`
//! facade. The library never installs a tracing subscriber or a metrics recorder: that is the application's choice.

use super::Error;
use crate::api::WorkflowId;
use serde_json::Value;
use std::future::Future;
use std::time::Duration;
use tracing::instrument::WithSubscriber;

/// The subscriber that is current here, unless there is none (then a global subscriber, even one installed later,
/// applies when the spawned work logs).
pub(crate) fn current_dispatch() -> Option<tracing::Dispatch> {
    tracing::dispatcher::get_default(|dispatch| {
        (!dispatch.is::<tracing::subscriber::NoSubscriber>()).then(|| dispatch.clone())
    })
}

/// `tokio::spawn`, keeping the current subscriber for the spawned work (spawned tasks do not inherit it otherwise).
pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match current_dispatch() {
        Some(dispatch) => tokio::spawn(future.with_subscriber(dispatch)),
        None => tokio::spawn(future),
    }
}

/// Wraps `f` to run inside the current span with the current subscriber, wherever (on whichever thread) it is called.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn carry<R>(f: impl FnOnce() -> R) -> impl FnOnce() -> R {
    let span = tracing::Span::current();
    let dispatch = current_dispatch();
    move || match dispatch {
        Some(dispatch) => tracing::dispatcher::with_default(&dispatch, || span.in_scope(f)),
        None => span.in_scope(f),
    }
}

/// `tokio::spawn` for work that belongs to what is happening here: it keeps the current span as well as the
/// subscriber, so its logs and spans stay in the same trace. (`spawn` is for work that outlives it, such as servers.)
#[cfg_attr(not(feature = "yaml"), allow(dead_code))]
pub(crate) fn spawn_in_span<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn(tracing::Instrument::in_current_span(future))
}

/// Runs `f` on the blocking pool inside the current span, with the current subscriber (on wasm, where there is no pool,
/// right here).
pub(crate) async fn spawn_blocking<F, R>(f: F) -> Result<R, crate::blocking::Error>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    crate::blocking::run(f).await
}

// ---- spans ----
//
// `tracing` span names are static, so each span also sets `otel.name`: `tracing-opentelemetry` exports that as the span's
// name, which is what a trace viewer such as Grafana shows. Failures set `otel.status_code` and `otel.status_message`.
// Every span the engine opens is made here, so names and fields stay consistent.

/// Which engine runs an instance.
#[derive(Debug, Clone, Copy)]
pub(crate) enum EngineKind {
    Native,
    #[cfg_attr(not(feature = "yaml"), allow(dead_code))]
    Yaml,
}

impl EngineKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Yaml => "yaml",
        }
    }
}

/// Where a new instance's span goes in the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Parent {
    /// Under whatever span is current (the caller's, or the parent's step for a child workflow).
    Current,
    /// A trace of its own that follows from the current span (`ctx.start`: the started workflow is on its own).
    Detached,
}

/// What a workflow span says about its instance.
pub(crate) struct WorkflowSpan<'a> {
    pub workflow: &'a WorkflowId,
    pub instance: &'a str,
    /// True when the instance resumes from an existing journal.
    pub replay: bool,
    pub parent_instance: Option<&'a str>,
    pub run: u32,
    pub engine: EngineKind,
    pub parent: Parent,
}

macro_rules! workflow_span {
    ($($parent:tt)*) => {
        |info: &WorkflowSpan<'_>| tracing::info_span!(
            $($parent)*
            "workflow",
            otel.name = %format_args!("workflow {}", info.workflow),
            workflow.id = %info.workflow,
            workflow.namespace = %info.workflow.namespace,
            workflow.name = %info.workflow.name,
            workflow.version = %info.workflow.version,
            workflow.engine = info.engine.as_str(),
            workflow.run = info.run.max(1),
            workflow.outcome = tracing::field::Empty,
            instance.id = %info.instance,
            parent.instance.id = info.parent_instance,
            replay = info.replay,
            otel.status_code = tracing::field::Empty,
            otel.status_message = tracing::field::Empty,
            error.type = tracing::field::Empty,
            exception.message = tracing::field::Empty,
        )
    };
}

/// The span an instance runs in.
pub(crate) fn workflow_span(info: &WorkflowSpan<'_>) -> tracing::Span {
    match info.parent {
        Parent::Current => workflow_span!()(info),
        Parent::Detached => {
            let span = workflow_span!(parent: None,)(info);
            span.follows_from(tracing::Span::current());
            span
        }
    }
}

/// Records how an instance ended on its span.
pub(crate) fn workflow_outcome(span: &tracing::Span, result: &Result<Value, Error>) {
    let outcome = match result {
        Ok(_) => "completed",
        Err(Error::Cancelled) => "cancelled",
        Err(_) => "failed",
    };
    span.record("workflow.outcome", outcome);
    match result {
        Ok(_) | Err(Error::Cancelled) => {}
        Err(error) => record_error(span, error_type(error), &error.to_string()),
    }
}

/// A short, stable name for the kind of an error, for `error.type`.
pub(crate) fn error_type(error: &Error) -> &str {
    match error {
        Error::Faulted(fault) => &fault.kind,
        Error::Cancelled => "cancelled",
        Error::Timeout { .. } => "timeout",
        Error::Invalid(_) => "invalid",
        Error::NonDeterminism(_) => "non_determinism",
        Error::Persistence(_) => "persistence",
        Error::Cache(_) => "cache",
        Error::Engine(_) => "engine",
        Error::Rejected(_) => "rejected",
    }
}

/// Marks `span` as failed.
pub(crate) fn record_error(span: &tracing::Span, kind: &str, message: &str) {
    span.record("otel.status_code", "ERROR");
    span.record("otel.status_message", message);
    span.record("error.type", kind);
    span.record("exception.message", message);
}

/// What a step span says about its step.
pub(crate) struct StepSpan<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub seq: usize,
    pub ordinal: usize,
    pub key: &'a str,
    pub location: &'a std::panic::Location<'a>,
    pub run: u32,
    /// `sleep`'s duration, for its name.
    pub sleep: Option<Duration>,
    pub timeout: Option<Duration>,
    pub shielded: bool,
}

/// The span name of a step: what it does and to what, such as `call charge` or `sleep 5s`.
fn step_title(step: &StepSpan<'_>) -> String {
    match (step.kind, step.sleep) {
        ("sleep", Some(duration)) => format!("sleep {duration:?}"),
        ("event", _) => format!("wait {}", step.name),
        (kind, _) => format!("{kind} {}", step.name),
    }
}

/// The span a live step runs in, from its start to the record of its outcome. Task steps record each retry's
/// `attempt`.
pub(crate) fn step_span(step: &StepSpan<'_>) -> tracing::Span {
    let span = tracing::info_span!(
        "step",
        otel.name = %step_title(step),
        step.name = %step.name,
        step.kind = %step.kind,
        step.seq = step.seq,
        step.ordinal = step.ordinal,
        step.key = %step.key,
        step.shielded = step.shielded,
        step.timeout_ms = tracing::field::Empty,
        step.outcome = tracing::field::Empty,
        step.input.bytes = tracing::field::Empty,
        step.input = tracing::field::Empty,
        step.output.bytes = tracing::field::Empty,
        step.output = tracing::field::Empty,
        workflow.run = step.run.max(1),
        attempt = 1u32,
        cache.hit = tracing::field::Empty,
        child.instance.id = tracing::field::Empty,
        code.filepath = step.location.file(),
        code.lineno = step.location.line(),
        otel.status_code = tracing::field::Empty,
        otel.status_message = tracing::field::Empty,
        error.type = tracing::field::Empty,
        exception.message = tracing::field::Empty,
    );
    if let Some(timeout) = step.timeout {
        span.record("step.timeout_ms", duration_ms(timeout));
    }
    span
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// How a step ended, for its span.
pub(crate) enum StepEnd<'a> {
    Ok(&'a Value),
    Fault(&'a super::Fault),
    Timeout(&'a str),
    Cancelled,
    Error(&'a Error),
}

impl StepEnd<'_> {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Ok(_) => "ok",
            Self::Fault(_) => "fault",
            Self::Timeout(_) => "timeout",
            Self::Cancelled => "cancelled",
            Self::Error(_) => "error",
        }
    }
}

/// Records how a step ended on its span (and its output, with payloads on).
pub(crate) fn step_outcome(span: &tracing::Span, end: &StepEnd<'_>, payloads: Payloads) {
    span.record("step.outcome", end.as_str());
    match end {
        StepEnd::Ok(output) => payloads.record(span, "step.output", output),
        StepEnd::Fault(fault) => record_error(span, &fault.kind, &fault.to_string()),
        StepEnd::Timeout(detail) => record_error(span, "timeout", detail),
        StepEnd::Cancelled => {}
        StepEnd::Error(error) => record_error(span, error_type(error), &error.to_string()),
    }
}

/// Whether, and how much of, step inputs and outputs go on spans (see `EngineBuilder::trace_payloads`). Their sizes
/// always do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Payloads {
    pub max_bytes: Option<usize>,
}

impl Payloads {
    /// Records `value`'s size as `{field}.bytes` and, with payloads on, the value itself as `field` (truncated).
    pub(crate) fn record(self, span: &tracing::Span, field: &str, value: &Value) {
        if span.is_disabled() {
            return;
        }
        let json = value.to_string();
        span.record(format!("{field}.bytes").as_str(), json.len());
        if let Some(max) = self.max_bytes {
            span.record(field, truncate(&json, max).as_ref());
        }
    }
}

/// `text`, cut to at most `max` bytes (on a character boundary) with a note of how long it was.
pub(crate) fn truncate(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if text.len() <= max {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let kept = text.get(..end).unwrap_or_default();
    std::borrow::Cow::Owned(format!("{kept}… ({} bytes)", text.len()))
}

/// One try of a task step, inside its step span.
pub(crate) fn attempt_span(task: &str, attempt: u32, idempotency_key: &str) -> tracing::Span {
    tracing::info_span!(
        "attempt",
        otel.name = %format_args!("attempt {attempt}"),
        task.name = %task,
        attempt,
        task.idempotency_key = %idempotency_key,
        attempt.outcome = tracing::field::Empty,
        attempt.retryable = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
        otel.status_message = tracing::field::Empty,
        error.type = tracing::field::Empty,
        exception.message = tracing::field::Empty,
    )
}

/// The wait between a failed attempt and the next.
pub(crate) fn backoff_span(task: &str, attempt: u32, delay: Duration) -> tracing::Span {
    tracing::info_span!(
        "retry.backoff",
        otel.name = %format_args!("retry backoff {delay:?}"),
        task.name = %task,
        retry.after_attempt = attempt,
        retry.delay_ms = duration_ms(delay),
    )
}

/// A task call waiting for a permit of its limit (only when it has to wait).
pub(crate) fn limit_wait_span(task: &str, limit: &str) -> tracing::Span {
    tracing::info_span!(
        "limit.wait",
        otel.name = %format_args!("wait for limit {limit}"),
        task.name = %task,
        limit.name = %limit,
    )
}

/// Writing an instance's journal changes to the store.
pub(crate) fn persist_span(instance: &str) -> tracing::Span {
    tracing::info_span!(
        "persist",
        otel.name = "persist journal",
        instance.id = %instance,
        journal.records = tracing::field::Empty,
    )
}

/// Steps an instance replayed from its journal instead of running them, from the first to the first live step. Each
/// replayed step is an event in it.
pub(crate) fn replay_span(instance: &str) -> tracing::Span {
    tracing::info_span!(
        "replay",
        otel.name = "replay",
        instance.id = %instance,
        replay.steps = 0usize,
    )
}

/// Notes a replayed step in the replay span.
pub(crate) fn step_replayed(
    span: &tracing::Span,
    steps: usize,
    kind: &str,
    name: &str,
    outcome: &str,
) {
    span.record("replay.steps", steps);
    let noun = if steps == 1 { "step" } else { "steps" };
    span.record("otel.name", format!("replay {steps} {noun}").as_str());
    span.in_scope(|| {
        tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO,
            step.kind = %kind,
            step.name = %name,
            step.outcome = %outcome,
            "step replayed"
        );
    });
}

/// A message to an instance from outside it (`kind` is `signal`, `update`, `query` or `event`).
pub(crate) fn message_span(kind: &'static str, name: &str, instance: &str) -> tracing::Span {
    tracing::info_span!(
        "message",
        otel.name = %format_args!("{kind} {name}"),
        message.kind = kind,
        message.name = %name,
        instance.id = %instance,
        message.remote = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
        otel.status_message = tracing::field::Empty,
        error.type = tracing::field::Empty,
        exception.message = tracing::field::Empty,
    )
}

/// A schedule firing the tick due `at`, `lag` late: the instance it starts runs inside it.
pub(crate) fn schedule_span(
    schedule: &str,
    at: chrono::DateTime<chrono::Utc>,
    lag: Duration,
) -> tracing::Span {
    tracing::info_span!(
        parent: None,
        "schedule.fire",
        otel.name = %format_args!("schedule {schedule}"),
        schedule.id = %schedule,
        schedule.due = %at.to_rfc3339(),
        schedule.lag_ms = duration_ms(lag),
        schedule.result = tracing::field::Empty,
        instance.id = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
        otel.status_message = tracing::field::Empty,
        error.type = tracing::field::Empty,
        exception.message = tracing::field::Empty,
    )
}

/// Lease upkeep (debug level: it runs all the time).
pub(crate) fn lease_span(operation: &'static str, engine: &str) -> tracing::Span {
    tracing::debug_span!(
        parent: None,
        "lease",
        otel.name = %format_args!("lease {operation}"),
        lease.operation = operation,
        engine.id = %engine,
        lease.instances = tracing::field::Empty,
        lease.late_ms = tracing::field::Empty,
    )
}

/// An engine-level operation (`recover instances`, `verify replay`, `run task`, ...).
pub(crate) fn engine_span(operation: &'static str, subject: &str) -> tracing::Span {
    tracing::info_span!(
        "engine",
        otel.name = %format_args!("{operation} {subject}"),
        engine.operation = operation,
        engine.subject = %subject,
        engine.count = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
        otel.status_message = tracing::field::Empty,
        error.type = tracing::field::Empty,
        exception.message = tracing::field::Empty,
    )
}

/// A YAML task, at `reference` (a JSON pointer into the workflow, such as `/do/1/check/do/0/fetch`).
#[cfg_attr(not(feature = "yaml"), allow(dead_code))]
pub(crate) fn yaml_task_span(
    name: &str,
    task_type: &str,
    reference: &str,
    instance: &str,
) -> tracing::Span {
    tracing::info_span!(
        "task",
        otel.name = %format_args!("{task_type} {name}"),
        task.name = %name,
        task.type = %task_type,
        task.reference = %reference,
        instance.id = %instance,
        task.call = tracing::field::Empty,
        task.outcome = tracing::field::Empty,
        cache.hit = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
        otel.status_message = tracing::field::Empty,
        error.type = tracing::field::Empty,
        exception.message = tracing::field::Empty,
    )
}

/// One pass of a YAML `for` loop.
#[cfg_attr(not(feature = "yaml"), allow(dead_code))]
pub(crate) fn iteration_span(index: usize, item_name: &str) -> tracing::Span {
    tracing::info_span!(
        "iteration",
        otel.name = %format_args!("iteration {index}"),
        for.index = index,
        for.each = %item_name,
    )
}

/// How a YAML task ended (`failure` is its error's kind and message), after `elapsed`.
#[cfg_attr(not(feature = "yaml"), allow(dead_code))]
pub(crate) fn yaml_task_ended(
    span: &tracing::Span,
    name: &str,
    task_type: &'static str,
    failure: Option<(&str, &str)>,
    elapsed: Duration,
) {
    let outcome = match failure {
        None => "ok",
        Some((kind, message)) => {
            record_error(span, kind, message);
            "fault"
        }
    };
    span.record("task.outcome", outcome);
    step_finished(name, task_type, outcome, elapsed);
}

/// Where an [`Event`](super::Event) is logged, as a span event of the span it happened in.
pub(crate) const EVENT_TARGET: &str = "jackdaw::event";

/// Logs `event` in the current span, so every event of an instance shows on its trace where it happened.
pub(crate) fn event_logged(event: &super::Event) {
    use super::Event;
    match event {
        Event::WorkflowStarted {
            instance_id,
            workflow,
            ..
        } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "WorkflowStarted", instance.id = %instance_id, workflow.id = %workflow,
            "workflow started"
        ),
        Event::WorkflowResumed { instance_id, .. } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "WorkflowResumed", instance.id = %instance_id, "workflow resumed"
        ),
        Event::TaskStarted {
            task_name,
            replayed,
            ..
        } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "TaskStarted", task.name = %task_name, replayed, "task started"
        ),
        Event::TaskCompleted {
            task_name,
            replayed,
            ..
        } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "TaskCompleted", task.name = %task_name, replayed, "task completed"
        ),
        Event::TaskFaulted {
            task_name,
            error,
            replayed,
            ..
        } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "TaskFaulted", task.name = %task_name, replayed,
            exception.message = %error, "task faulted"
        ),
        Event::TaskRetried {
            task_name, attempt, ..
        } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "TaskRetried", task.name = %task_name, attempt, "task retried"
        ),
        Event::WorkflowCompleted { .. } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "WorkflowCompleted", "workflow completed"
        ),
        Event::WorkflowFailed { error, .. } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "WorkflowFailed", exception.message = %error, "workflow failed"
        ),
        Event::WorkflowCancelled { .. } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "WorkflowCancelled", "workflow cancelled"
        ),
        Event::ContinuedAsNew { run, .. } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "ContinuedAsNew", workflow.run = run, "continued as new"
        ),
        Event::Emitted { event, .. } => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = "Emitted", cloudevent.type = %event.event_type,
            cloudevent.id = %event.id, "event emitted"
        ),
        Event::Other(other) => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO, event.name = other.kind(), "yaml event"
        ),
    }
}

#[cfg(feature = "metrics")]
fn workflow_label(workflow: &WorkflowId) -> String {
    workflow.to_string()
}

/// A new instance started.
pub(crate) fn workflow_started(workflow: &WorkflowId) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_workflows_started_total", "workflow" => workflow_label(workflow))
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = workflow;
}

/// A stopped instance was picked up again from its journal.
pub(crate) fn workflow_resumed(workflow: &WorkflowId) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_workflows_resumed_total", "workflow" => workflow_label(workflow))
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = workflow;
}

/// An instance ended with `result`, `elapsed` after this engine started (or resumed) it.
pub(crate) fn workflow_ended(
    workflow: &WorkflowId,
    result: &Result<Value, Error>,
    elapsed: Duration,
) {
    #[cfg(feature = "metrics")]
    {
        let (name, outcome) = match result {
            Ok(_) => ("jackdaw_workflows_completed_total", "completed"),
            Err(Error::Cancelled) => ("jackdaw_workflows_cancelled_total", "cancelled"),
            Err(_) => ("jackdaw_workflows_failed_total", "failed"),
        };
        metrics::counter!(name, "workflow" => workflow_label(workflow)).increment(1);
        metrics::histogram!(
            "jackdaw_workflow_duration_seconds",
            "workflow" => workflow_label(workflow),
            "outcome" => outcome
        )
        .record(elapsed.as_secs_f64());
    }
    #[cfg(not(feature = "metrics"))]
    let _ = (workflow, result, elapsed);
}

/// Counts an instance of `workflow` as running in this engine until dropped.
pub(crate) fn workflow_running(workflow: &WorkflowId) -> Running {
    #[cfg(feature = "metrics")]
    {
        let label = workflow_label(workflow);
        metrics::gauge!("jackdaw_workflows_running", "workflow" => label.clone()).increment(1.0);
        Running { label }
    }
    #[cfg(not(feature = "metrics"))]
    {
        let _ = workflow;
        Running {}
    }
}

/// See [`workflow_running`].
pub(crate) struct Running {
    #[cfg(feature = "metrics")]
    label: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        #[cfg(feature = "metrics")]
        metrics::gauge!("jackdaw_workflows_running", "workflow" => self.label.clone())
            .decrement(1.0);
    }
}

/// A live step finished with `outcome` after `elapsed`.
pub(crate) fn step_finished(name: &str, kind: &str, outcome: &'static str, elapsed: Duration) {
    #[cfg(feature = "metrics")]
    metrics::histogram!(
        "jackdaw_step_duration_seconds",
        "step" => name.to_string(),
        "kind" => kind.to_string(),
        "outcome" => outcome
    )
    .record(elapsed.as_secs_f64());
    #[cfg(not(feature = "metrics"))]
    let _ = (name, kind, outcome, elapsed);
}

/// A step was replayed from the journal instead of run.
pub(crate) fn step_replayed_metric(workflow: &WorkflowId) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_steps_replayed_total", "workflow" => workflow_label(workflow))
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = workflow;
}

/// A task step looked its result up in the cache.
pub(crate) fn cache_lookup(task: &str, hit: bool) {
    #[cfg(feature = "metrics")]
    metrics::counter!(
        "jackdaw_task_cache_requests_total",
        "task" => task.to_string(),
        "result" => if hit { "hit" } else { "miss" }
    )
    .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = (task, hit);
}

/// A task attempt sent no heartbeat in time.
pub(crate) fn heartbeat_timed_out(task: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_task_heartbeat_timeouts_total", "task" => task.to_string())
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = task;
}

/// One attempt of a task ended after `elapsed`: `Ok`, or its fault and whether it may be retried.
pub(crate) fn attempt_ended(
    span: &tracing::Span,
    task: &str,
    result: Result<(), (&super::Fault, bool)>,
    elapsed: Duration,
) {
    let outcome = match result {
        Ok(()) => "ok",
        Err((_, true)) => "retryable_error",
        Err((_, false)) => "error",
    };
    span.record("attempt.outcome", outcome);
    if let Err((fault, retryable)) = result {
        span.record("attempt.retryable", retryable);
        record_error(span, &fault.kind, &fault.to_string());
    }
    #[cfg(feature = "metrics")]
    metrics::histogram!(
        "jackdaw_task_attempt_duration_seconds",
        "task" => task.to_string(),
        "outcome" => outcome
    )
    .record(elapsed.as_secs_f64());
    #[cfg(not(feature = "metrics"))]
    let _ = (task, elapsed);
}

/// A task call waited `elapsed` for a permit of the limit `name`.
pub(crate) fn limit_waited(name: &str, elapsed: Duration) {
    #[cfg(feature = "metrics")]
    metrics::histogram!("jackdaw_task_limit_wait_seconds", "limit" => name.to_string())
        .record(elapsed.as_secs_f64());
    #[cfg(not(feature = "metrics"))]
    let _ = (name, elapsed);
}

/// An instance started its next run with `continue_as_new`.
pub(crate) fn continued_as_new(workflow: &WorkflowId) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_workflows_continued_as_new_total", "workflow" => workflow_label(workflow))
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = workflow;
}

/// A signal, update, query or event (`kind`) reached an instance, or failed to (marked on its span).
pub(crate) fn message_delivered<T>(
    span: &tracing::Span,
    kind: &'static str,
    result: &Result<T, Error>,
) {
    if let Err(error) = result {
        record_error(span, error_type(error), &error.to_string());
    }
    #[cfg(feature = "metrics")]
    metrics::counter!(
        "jackdaw_messages_delivered_total",
        "kind" => kind,
        "result" => if result.is_ok() { "ok" } else { "error" }
    )
    .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = kind;
}

/// A schedule's tick fired with `result` (`started`, `already_started`, `skipped_overlap` or `error`), `lag` after it
/// was due.
pub(crate) fn schedule_fired(schedule: &str, result: &'static str, lag: Duration) {
    #[cfg(feature = "metrics")]
    {
        metrics::counter!(
            "jackdaw_schedule_fires_total",
            "schedule" => schedule.to_string(),
            "result" => result
        )
        .increment(1);
        metrics::histogram!("jackdaw_schedule_lag_seconds", "schedule" => schedule.to_string())
            .record(lag.as_secs_f64());
    }
    #[cfg(not(feature = "metrics"))]
    let _ = (schedule, result, lag);
}

/// The lease upkeep loop woke `late` after it meant to.
pub(crate) fn lease_tended(late: Duration) {
    #[cfg(feature = "metrics")]
    metrics::histogram!("jackdaw_lease_renewal_lateness_seconds").record(late.as_secs_f64());
    #[cfg(not(feature = "metrics"))]
    let _ = late;
}

/// One instance's lease renewal ended with `result` (`renewed`, `lost` or `error`).
pub(crate) fn lease_renewed(result: &'static str) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_lease_renewals_total", "result" => result).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = result;
}

/// How many instances this engine holds leases on.
pub(crate) fn leases_held(count: usize) {
    #[cfg(feature = "metrics")]
    #[allow(clippy::cast_precision_loss)]
    metrics::gauge!("jackdaw_leases_held").set(count as f64);
    #[cfg(not(feature = "metrics"))]
    let _ = count;
}

/// Another engine took an instance of `workflow` over.
pub(crate) fn lease_lost(workflow: &WorkflowId) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_leases_lost_total", "workflow" => workflow_label(workflow))
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = workflow;
}

/// Replaying an instance of `workflow` (named as in its divergence) found code that no longer matches its history.
pub(crate) fn diverged(workflow: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_replay_divergences_total", "workflow" => workflow.to_string())
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = workflow;
}

/// A task attempt failed and will be retried.
pub(crate) fn task_retried(name: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_task_retries_total", "task" => name.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = name;
}

/// Counts a call waiting for the limit `name` until the guard is dropped.
pub(crate) fn limit_waiting(name: &str) -> LimitWaiting {
    #[cfg(feature = "metrics")]
    metrics::gauge!("jackdaw_task_limit_waiting", "limit" => name.to_string()).increment(1.0);
    #[cfg(not(feature = "metrics"))]
    let _ = name;
    LimitWaiting {
        #[cfg(feature = "metrics")]
        name: name.to_string(),
    }
}

/// See [`limit_waiting`].
pub(crate) struct LimitWaiting {
    #[cfg(feature = "metrics")]
    name: String,
}

impl Drop for LimitWaiting {
    fn drop(&mut self) {
        #[cfg(feature = "metrics")]
        metrics::gauge!("jackdaw_task_limit_waiting", "limit" => self.name.clone()).decrement(1.0);
    }
}

// ---- the store ----

/// A store that reports each call: a debug-level span (`store {operation}`), its latency and failures as metrics, and,
/// with `log_events`, every event saved as a span event of the span it was saved in (the YAML engine's events, which
/// it saves straight to the store).
#[derive(Debug)]
pub(crate) struct TracedStore {
    inner: std::sync::Arc<dyn crate::PersistenceProvider>,
    backend: &'static str,
    log_events: bool,
}

/// Wraps `store` in a [`TracedStore`], unless it is one already.
pub(crate) fn traced_store(
    store: std::sync::Arc<dyn crate::PersistenceProvider>,
    log_events: bool,
) -> std::sync::Arc<dyn crate::PersistenceProvider> {
    if store.traced() {
        return store;
    }
    // The type's own name, without its path or generics: `RedbPersistence`, `LeaseFence`, ...
    let full = std::any::type_name_of_val(&*store);
    let base = full.split('<').next().unwrap_or(full);
    let backend = base.rsplit("::").next().unwrap_or(base);
    std::sync::Arc::new(TracedStore {
        inner: store,
        backend,
        log_events,
    })
}

impl TracedStore {
    async fn timed<T>(
        &self,
        operation: &'static str,
        call: impl Future<Output = crate::persistence::Result<T>>,
    ) -> crate::persistence::Result<T> {
        let span = tracing::debug_span!(
            "store",
            otel.name = %format_args!("store {operation}"),
            db.operation.name = operation,
            db.system.name = self.backend,
            otel.status_code = tracing::field::Empty,
            otel.status_message = tracing::field::Empty,
        );
        let started = std::time::Instant::now();
        let result = tracing::Instrument::instrument(call, span.clone()).await;
        if let Err(error) = &result {
            span.record("otel.status_code", "ERROR");
            span.record("otel.status_message", error.to_string().as_str());
        }
        store_called(operation, self.backend, result.is_ok(), started.elapsed());
        result
    }
}

/// A store call took `elapsed` (and failed, unless `ok`).
fn store_called(operation: &'static str, backend: &'static str, ok: bool, elapsed: Duration) {
    #[cfg(feature = "metrics")]
    {
        metrics::histogram!(
            "jackdaw_persistence_operation_duration_seconds",
            "operation" => operation,
            "backend" => backend
        )
        .record(elapsed.as_secs_f64());
        if !ok {
            metrics::counter!(
                "jackdaw_persistence_errors_total",
                "operation" => operation,
                "backend" => backend
            )
            .increment(1);
        }
    }
    #[cfg(not(feature = "metrics"))]
    let _ = (operation, backend, ok, elapsed);
}

/// Logs a YAML engine event in the current span.
pub(crate) fn stored_event_logged(event: &crate::workflow::WorkflowEvent) {
    match event.task_name() {
        Some(task) => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO,
            event.name = event.kind(),
            task.name = %task,
            instance.id = %event.instance_id(),
            "{}",
            event.kind()
        ),
        None => tracing::event!(
            target: EVENT_TARGET,
            tracing::Level::INFO,
            event.name = event.kind(),
            instance.id = %event.instance_id(),
            "{}",
            event.kind()
        ),
    }
}

#[async_trait::async_trait]
impl crate::PersistenceProvider for TracedStore {
    async fn save_event(
        &self,
        event: crate::workflow::WorkflowEvent,
    ) -> crate::persistence::Result<()> {
        if self.log_events {
            stored_event_logged(&event);
        }
        self.timed("save_event", self.inner.save_event(event)).await
    }

    async fn get_events(
        &self,
        instance_id: &str,
    ) -> crate::persistence::Result<Vec<crate::workflow::WorkflowEvent>> {
        self.timed("get_events", self.inner.get_events(instance_id))
            .await
    }

    async fn save_checkpoint(
        &self,
        checkpoint: crate::workflow::WorkflowCheckpoint,
    ) -> crate::persistence::Result<()> {
        self.timed("save_checkpoint", self.inner.save_checkpoint(checkpoint))
            .await
    }

    async fn get_checkpoint(
        &self,
        instance_id: &str,
    ) -> crate::persistence::Result<Option<crate::workflow::WorkflowCheckpoint>> {
        self.timed("get_checkpoint", self.inner.get_checkpoint(instance_id))
            .await
    }

    async fn append_journal(
        &self,
        instance_id: &str,
        records: Vec<crate::persistence::JournalRecord>,
    ) -> crate::persistence::Result<()> {
        self.timed(
            "append_journal",
            self.inner.append_journal(instance_id, records),
        )
        .await
    }

    async fn load_journal(
        &self,
        instance_id: &str,
    ) -> crate::persistence::Result<Vec<crate::persistence::JournalRecord>> {
        self.timed("load_journal", self.inner.load_journal(instance_id))
            .await
    }

    async fn prune_journal(
        &self,
        instance_id: &str,
        before: u64,
    ) -> crate::persistence::Result<()> {
        self.timed(
            "prune_journal",
            self.inner.prune_journal(instance_id, before),
        )
        .await
    }

    async fn create_checkpoint(
        &self,
        checkpoint: crate::workflow::WorkflowCheckpoint,
    ) -> crate::persistence::Result<bool> {
        self.timed(
            "create_checkpoint",
            self.inner.create_checkpoint(checkpoint),
        )
        .await
    }

    async fn list_instances(&self) -> crate::persistence::Result<Option<Vec<String>>> {
        self.timed("list_instances", self.inner.list_instances())
            .await
    }

    async fn delete_instance(&self, instance_id: &str) -> crate::persistence::Result<()> {
        self.timed("delete_instance", self.inner.delete_instance(instance_id))
            .await
    }

    fn supports_leases(&self) -> bool {
        self.inner.supports_leases()
    }

    async fn acquire_lease(
        &self,
        instance_id: &str,
        owner: &str,
        now: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> crate::persistence::Result<crate::persistence::Lease> {
        self.timed(
            "acquire_lease",
            self.inner.acquire_lease(instance_id, owner, now, until),
        )
        .await
    }

    async fn release_lease(
        &self,
        instance_id: &str,
        owner: &str,
    ) -> crate::persistence::Result<()> {
        self.timed(
            "release_lease",
            self.inner.release_lease(instance_id, owner),
        )
        .await
    }

    async fn post_message(
        &self,
        instance_id: &str,
        message: serde_json::Value,
    ) -> crate::persistence::Result<()> {
        self.timed(
            "post_message",
            self.inner.post_message(instance_id, message),
        )
        .await
    }

    async fn take_messages(
        &self,
        owner: &str,
    ) -> crate::persistence::Result<Vec<(String, serde_json::Value)>> {
        self.timed("take_messages", self.inner.take_messages(owner))
            .await
    }

    fn traced(&self) -> bool {
        true
    }
}
