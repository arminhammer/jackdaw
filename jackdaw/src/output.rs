//! The YAML interpreter's trace, as [`tracing`] events on the [`TARGET`] target.
//!
//! Each event has an `event` field naming what happened (`task_start`, `task_output`, ...) and the values a renderer
//! needs; JSON values are fields holding compact JSON text. The trace is at `DEBUG` (it carries inputs and outputs),
//! except the lines a script prints (`script_line`) and listener startup (`listener_started`), which are at `INFO`. The
//! `jackdaw` CLI renders them; a library user sees them only through their own subscriber.

use serde_json::Value;

/// The target of every trace event.
pub const TARGET: &str = "jackdaw::yaml";

/// Displays a JSON value as compact JSON text, for an event field.
struct Json<'a>(&'a Value);

impl std::fmt::Display for Json<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self.0).unwrap_or_else(|_| "null".to_string()))
    }
}

/// A workflow instance resumed from its checkpoint.
pub fn format_workflow_resume(instance_id: &str, from_task: Option<&str>) {
    tracing::debug!(target: TARGET, event = "workflow_resume", instance = instance_id, from_task);
}

/// A workflow finished with `output`.
pub fn format_workflow_output(output: &Value, duration_ms: i64) {
    tracing::debug!(
        target: TARGET,
        event = "workflow_output",
        output = %Json(&filter_internal_fields(output)),
        duration_ms
    );
}

/// A task started.
pub fn format_task_start(task_name: &str, task_type: &str) {
    tracing::debug!(target: TARGET, event = "task_start", task = task_name, kind = task_type);
}

/// A task was skipped because its checkpoint says it already completed.
pub fn format_task_skipped(task_name: &str) {
    tracing::debug!(target: TARGET, event = "task_skipped", task = task_name);
}

/// A task's result came from the cache.
pub fn format_cache_hit(task_name: &str, key: &str, timestamp: Option<&str>) {
    tracing::debug!(
        target: TARGET,
        event = "cache_hit",
        task = task_name,
        key,
        cached_at = timestamp
    );
}

/// A task's result was not in the cache.
pub fn format_cache_miss(task_name: &str, key: &str) {
    tracing::debug!(target: TARGET, event = "cache_miss", task = task_name, key);
}

/// Filter out internal descriptor fields from display
#[must_use]
pub fn filter_internal_fields(value: &Value) -> Value {
    if let Some(obj) = value.as_object() {
        let filtered: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(key, _)| !key.starts_with("__"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Value::Object(filtered)
    } else {
        value.clone()
    }
}

/// The context a task starts with.
pub fn format_task_context(context: &Value) {
    tracing::debug!(
        target: TARGET,
        event = "task_context",
        context = %Json(&filter_internal_fields(context))
    );
}

/// The input a task starts with.
pub fn format_task_input(input: &Value) {
    tracing::debug!(
        target: TARGET,
        event = "task_input",
        input = %Json(&filter_internal_fields(input))
    );
}

/// A `run` task's language, stdin, arguments and environment.
pub fn format_run_task_params(
    language: Option<&str>,
    stdin: Option<&str>,
    arguments: Option<&Value>,
    environment: Option<&Value>,
) {
    tracing::debug!(
        target: TARGET,
        event = "run_params",
        language,
        stdin,
        arguments = arguments.map(|value| tracing::field::display(Json(value))),
        environment = environment.map(|value| tracing::field::display(Json(value))),
    );
}

/// A task finished with `output`. Output a script streamed while it ran is marked `streamed`, so a renderer need not
/// show its stdout again.
pub fn format_task_output(output: &Value, duration_ms: i64) {
    let streamed = output
        .get("__streamed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    tracing::debug!(
        target: TARGET,
        event = "task_output",
        output = %Json(&filter_internal_fields(output)),
        duration_ms,
        streamed
    );
}

/// A line a script task printed while it ran (`stream` is `stdout` or `stderr`). `index` tells concurrent tasks
/// apart.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub fn script_line(task_name: &str, index: usize, stream: &str, line: &str) {
    tracing::info!(
        target: TARGET,
        event = "script_line",
        task = task_name,
        index,
        stream,
        line
    );
}

/// A `listen` task's server is accepting requests (`protocol` is `HTTP` or `gRPC`).
#[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
pub fn listener_started(protocol: &str, addr: &str) {
    tracing::info!(target: TARGET, event = "listener_started", protocol, addr);
}

/// A `listen` task's server routes `what` (`route /pets/{id}`, `gRPC method pets.Pets/Get`) to `task`.
#[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
pub fn listener_route(what: &str, task: &str, addr: &str) {
    tracing::info!(target: TARGET, event = "listener_route", route = what, task, addr);
}
