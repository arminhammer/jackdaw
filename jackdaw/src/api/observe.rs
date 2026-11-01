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
fn current_dispatch() -> Option<tracing::Dispatch> {
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

/// Runs `f` on the blocking pool inside the current span, with the current subscriber.
pub(crate) fn spawn_blocking<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let span = tracing::Span::current();
    let dispatch = current_dispatch();
    tokio::task::spawn_blocking(move || match dispatch {
        Some(dispatch) => tracing::dispatcher::with_default(&dispatch, || span.in_scope(f)),
        None => span.in_scope(f),
    })
}

/// The span an instance runs in. `replay` is true when it resumes from an existing journal.
pub(crate) fn workflow_span(workflow: &WorkflowId, instance: &str, replay: bool) -> tracing::Span {
    tracing::info_span!("workflow", workflow.id = %workflow, instance.id = %instance, replay)
}

/// The span a live step runs in. Task steps record each retry's `attempt`.
pub(crate) fn step_span(name: &str, kind: &str) -> tracing::Span {
    tracing::info_span!("step", step.name = %name, step.kind = %kind, attempt = 1u32)
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

/// An instance ended with `result`.
pub(crate) fn workflow_ended(workflow: &WorkflowId, result: &Result<Value, Error>) {
    #[cfg(feature = "metrics")]
    {
        let name = match result {
            Ok(_) => "jackdaw_workflows_completed_total",
            Err(Error::Cancelled) => "jackdaw_workflows_cancelled_total",
            Err(_) => "jackdaw_workflows_failed_total",
        };
        metrics::counter!(name, "workflow" => workflow_label(workflow)).increment(1);
    }
    #[cfg(not(feature = "metrics"))]
    let _ = (workflow, result);
}

/// A live step finished after `elapsed`.
pub(crate) fn step_finished(name: &str, kind: &str, elapsed: Duration) {
    #[cfg(feature = "metrics")]
    metrics::histogram!(
        "jackdaw_step_duration_seconds",
        "step" => name.to_string(),
        "kind" => kind.to_string()
    )
    .record(elapsed.as_secs_f64());
    #[cfg(not(feature = "metrics"))]
    let _ = (name, kind, elapsed);
}

/// A task attempt failed and will be retried.
pub(crate) fn task_retried(name: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!("jackdaw_task_retries_total", "task" => name.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = name;
}
