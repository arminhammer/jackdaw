use crate::dsl::task::TaskDefinition;
use crate::dsl::workflow::WorkflowDefinition;
use async_recursion::async_recursion;
use chrono::Utc;
use petgraph::{graph::DiGraph, stable_graph::NodeIndex};
use snafu::prelude::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[cfg(feature = "grpc-listener")]
use crate::listeners::grpc::GrpcListener;
#[cfg(feature = "openapi")]
use crate::providers::executors::OpenApiExecutor;
#[cfg(not(target_family = "wasm"))]
use crate::providers::executors::{PythonExecutor, RestExecutor, TypeScriptExecutor};
use crate::{
    context::Context, execution_handle::ExecutionHandle, executor::Executor, output,
    persistence::PersistenceProvider, workflow::WorkflowEvent,
};

use super::cache::CacheProvider;

/// The store streams that keep the events published on an engine (see [`DurableEngine::publish`]), one per day:
/// `__jackdaw_emitted__/2026-11-01`. They have events and no checkpoint, so they are never listed as instances, and a
/// day's stream is deleted whole once it is older than the engine's retention (see `Engine::sweep`).
pub(crate) const EMITTED_ID: &str = "__jackdaw_emitted__";

/// The stream for the events published on `day`.
fn emitted_stream(day: chrono::NaiveDate) -> String {
    format!("{EMITTED_ID}/{day}")
}

// Submodules
mod called;
mod catalog;
mod export;
mod graph;
mod listeners;
mod tasks;
mod timeout;

pub(crate) use called::{called_names, unknown_directives};

/// Why wasm cannot run a call type or script language, if that is why nothing runs it.
#[cfg(target_family = "wasm")]
fn wasm_limit(name: &str) -> Option<&'static str> {
    match name {
        "http" | "rest" | "openapi" => Some("has no HTTP client"),
        "python" | "javascript" => Some("cannot start processes"),
        _ => None,
    }
}

/// The error for a `call:` of a type nothing is registered for.
#[cfg(not(target_family = "wasm"))]
fn no_call_executor(call_type: &str) -> Error {
    Error::TaskExecution {
        message: format!("No executor for call type: {call_type}"),
    }
}

/// The error for a `call:` of a type nothing is registered for. On wasm, the ones that need an HTTP client or a process
/// say so.
#[cfg(target_family = "wasm")]
fn no_call_executor(call_type: &str) -> Error {
    Error::TaskExecution {
        message: match wasm_limit(call_type) {
            Some(reason) => {
                format!("`call: {call_type}` is not available on wasm, which {reason}")
            }
            None => format!("No executor for call type: {call_type}"),
        },
    }
}

/// The error for a `run: script` in a language nothing is registered for.
#[cfg(not(target_family = "wasm"))]
fn no_script_executor(language: &str) -> Error {
    Error::TaskExecution {
        message: format!("No executor found for language: {language}"),
    }
}

/// The error for a `run: script` in a language nothing is registered for. On wasm, the ones that need a process say so.
#[cfg(target_family = "wasm")]
fn no_script_executor(language: &str) -> Error {
    Error::TaskExecution {
        message: match wasm_limit(language) {
            Some(reason) => format!(
                "`run: script` language `{language}` is not available on wasm, which {reason}"
            ),
            None => format!("No executor found for language: {language}"),
        },
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("Workflow execution error: {message}"))]
    WorkflowExecution { message: String },

    #[snafu(display("Task execution error: {message}"))]
    TaskExecution { message: String },

    #[snafu(display("Listener error: {message}"))]
    Listener { message: String },

    #[snafu(display("Configuration error: {message}"))]
    Configuration { message: String },

    #[snafu(display("Timeout: {message}"))]
    Timeout { message: String },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("Executor error: {source}"))]
    Executor { source: crate::executor::Error },

    #[snafu(display("Persistence error: {source}"))]
    Persistence { source: crate::persistence::Error },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: crate::cache::Error },

    #[snafu(display("Context error: {source}"))]
    Context { source: crate::context::Error },

    #[snafu(display("Expression error: {source}"))]
    Expression { source: crate::expressions::Error },

    #[snafu(display("Serialization error: {source}"))]
    Serialization { source: serde_json::Error },

    #[snafu(display("Listener setup error: {source}"))]
    ListenerSetup { source: crate::listeners::Error },

    #[cfg(feature = "grpc-listener")]
    #[snafu(display("Protobuf compilation error: {source}"))]
    Protobuf { source: protox::Error },

    #[cfg(feature = "grpc-listener")]
    #[snafu(display("Protobuf descriptor error: {source}"))]
    ProtobufDescriptor {
        source: prost_reflect::DescriptorError,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

// Manual From implementations for error conversions
impl From<crate::persistence::Error> for Error {
    fn from(source: crate::persistence::Error) -> Self {
        Error::Persistence { source }
    }
}

impl From<crate::cache::Error> for Error {
    fn from(source: crate::cache::Error) -> Self {
        Error::Cache { source }
    }
}

impl From<crate::context::Error> for Error {
    fn from(source: crate::context::Error) -> Self {
        Error::Context { source }
    }
}

impl From<crate::expressions::Error> for Error {
    fn from(source: crate::expressions::Error) -> Self {
        Error::Expression { source }
    }
}

impl From<serde_json::Error> for Error {
    fn from(source: serde_json::Error) -> Self {
        Error::Serialization { source }
    }
}

impl From<crate::executor::Error> for Error {
    fn from(source: crate::executor::Error) -> Self {
        Error::Executor { source }
    }
}

impl From<crate::listeners::Error> for Error {
    fn from(source: crate::listeners::Error) -> Self {
        Error::ListenerSetup { source }
    }
}

#[cfg(feature = "grpc-listener")]
impl From<protox::Error> for Error {
    fn from(source: protox::Error) -> Self {
        Error::Protobuf { source }
    }
}

#[cfg(feature = "grpc-listener")]
impl From<prost_reflect::DescriptorError> for Error {
    fn from(source: prost_reflect::DescriptorError) -> Self {
        Error::ProtobufDescriptor { source }
    }
}

pub struct DurableEngine {
    executors: Arc<HashMap<String, Box<dyn Executor>>>,
    persistence: Arc<dyn PersistenceProvider>,
    cache: Arc<dyn CacheProvider>,
    /// Registry of active gRPC listeners, keyed by bind address
    /// Using `Arc<GrpcListener>` to allow adding methods progressively
    #[cfg(feature = "grpc-listener")]
    grpc_listeners: Arc<RwLock<HashMap<String, Arc<GrpcListener>>>>,
    /// Registry of active HTTP listeners, keyed by bind address
    /// Using `Arc<HttpListener>` to allow adding routes progressively
    #[cfg(feature = "http-listener")]
    http_listeners: Arc<RwLock<HashMap<String, Arc<crate::listeners::HttpListener>>>>,
    /// Registry of workflows for nested execution, keyed by "namespace/name/version"
    workflow_registry: Arc<RwLock<HashMap<String, WorkflowDefinition>>>,
    /// Event buffer size for streaming execution
    event_buffer_size: usize,
    /// The engine's clock: task start times, and when `wait` tasks end.
    clock: Arc<dyn crate::api::Clock>,
    /// Events emitted on this engine (by `emit` tasks and `ctx.emit`), for the `listen` tasks waiting for them.
    events: tokio::sync::broadcast::Sender<serde_json::Value>,
    /// The day up to which the streams of published events were deleted (see `prune_emitted`).
    emitted_pruned: std::sync::Mutex<Option<chrono::NaiveDate>>,
}

impl std::fmt::Debug for DurableEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableEngine")
            .field("executors", &self.executors.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl DurableEngine {
    /// Create a new ``DurableEngine`` instance
    ///
    /// # Errors
    /// Currently, this function does not return errors, but returns `Result` for future extensibility
    pub fn new(
        persistence: Arc<dyn PersistenceProvider>,
        cache: Arc<dyn CacheProvider>,
    ) -> Result<Self> {
        Self::new_with_config(persistence, cache, 1000)
    }

    /// Create a new ``DurableEngine`` instance with custom configuration
    ///
    /// This is used internally by the builder. Use [`crate::DurableEngineBuilder`] instead.
    ///
    /// # Errors
    /// Currently, this function does not return errors, but returns `Result` for future extensibility
    pub(crate) fn new_with_config(
        persistence: Arc<dyn PersistenceProvider>,
        cache: Arc<dyn CacheProvider>,
        event_buffer_size: usize,
    ) -> Result<Self> {
        #[cfg_attr(target_family = "wasm", allow(unused_mut))]
        let mut executors: HashMap<String, Box<dyn Executor>> = HashMap::new();
        // Native only: wasm has no HTTP client and no child processes, so `call: http` and the script languages
        // are not registered there (a task that names one fails with "No executor found").
        #[cfg(not(target_family = "wasm"))]
        executors.insert(
            "http".into(),
            Box::new(RestExecutor(reqwest::Client::new())),
        );
        #[cfg(not(target_family = "wasm"))]
        executors.insert(
            "rest".into(),
            Box::new(RestExecutor(reqwest::Client::new())),
        );
        #[cfg(feature = "openapi")]
        executors.insert(
            "openapi".into(),
            Box::new(OpenApiExecutor(reqwest::Client::new())),
        );
        #[cfg(not(target_family = "wasm"))]
        executors.insert("python".into(), Box::new(PythonExecutor::new()));
        #[cfg(not(target_family = "wasm"))]
        executors.insert("javascript".into(), Box::new(TypeScriptExecutor::new()));
        Ok(Self {
            executors: Arc::new(executors),
            persistence,
            cache,
            #[cfg(feature = "grpc-listener")]
            grpc_listeners: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "http-listener")]
            http_listeners: Arc::new(RwLock::new(HashMap::new())),
            workflow_registry: Arc::new(RwLock::new(HashMap::new())),
            event_buffer_size,
            clock: Arc::new(crate::api::SystemClock),
            events: tokio::sync::broadcast::channel(256).0,
            emitted_pruned: std::sync::Mutex::new(None),
        })
    }

    /// Delivers an emitted CloudEvent (as JSON) to the `listen` tasks waiting on this engine, and keeps it in the
    /// store's stream for the day (see [`EMITTED_ID`]) for `listen` tasks that resume after a restart. A listen task
    /// sees only events emitted after it first started.
    pub(crate) async fn publish(&self, event: serde_json::Value) {
        let now = self.clock.now();
        let stored = WorkflowEvent::EventEmitted {
            instance_id: emitted_stream(now.date_naive()),
            event: event.clone(),
            timestamp: now,
        };
        if let Err(error) = self.persistence.save_event(stored).await {
            tracing::warn!(%error, "an emitted event could not be stored: a listen task that resumes later will miss it");
        }
        let _ = self.events.send(event);
    }

    /// The events published on the engine since `since`, from the store, oldest first.
    pub(crate) async fn emitted_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<serde_json::Value>> {
        let mut events = Vec::new();
        let mut day = since.date_naive();
        let today = self.clock.now().date_naive();
        while day <= today {
            for stored in self.persistence.get_events(&emitted_stream(day)).await? {
                if let Some(event) = emitted_after(stored, since) {
                    events.push(event);
                }
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        Ok(events)
    }

    /// Deletes the streams of published events for the days before `before` (`Engine::sweep`, with the engine's
    /// retention). Looks back a year the first time, then only at the days since the last prune.
    pub(crate) async fn prune_emitted(
        &self,
        before: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize> {
        let end = before.date_naive();
        let start = {
            let pruned = lock_pruned(&self.emitted_pruned);
            pruned.unwrap_or_else(|| end.checked_sub_days(chrono::Days::new(366)).unwrap_or(end))
        };
        let mut deleted = 0;
        let mut day = start;
        while day < end {
            self.persistence
                .delete_instance(&emitted_stream(day))
                .await?;
            deleted += 1;
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        *lock_pruned(&self.emitted_pruned) = Some(end);
        Ok(deleted)
    }

    /// Uses the engine's clock (a test clock, for example) instead of the system clock.
    pub(crate) fn set_clock(&mut self, clock: Arc<dyn crate::api::Clock>) {
        self.clock = clock;
    }

    /// The YAML workflow registered under `namespace/name/version`.
    pub(crate) async fn registered(&self, key: &str) -> Option<WorkflowDefinition> {
        self.workflow_registry.read().await.get(key).cloned()
    }

    /// Adds executors (the engine's native tasks, for YAML workflows that `call:` them by name).
    ///
    /// Only valid before the engine is shared; a name that is already taken keeps its existing executor.
    pub(crate) fn add_executors(&mut self, extra: HashMap<String, Box<dyn Executor>>) {
        if let Some(executors) = Arc::get_mut(&mut self.executors) {
            for (name, executor) in extra {
                executors.entry(name).or_insert(executor);
            }
        }
    }

    /// Registers YAML workflows that others run by name (`run: workflow:`). Only valid before the engine is shared.
    pub(crate) fn add_workflows(&mut self, workflows: Vec<WorkflowDefinition>) {
        if let Some(registry) = Arc::get_mut(&mut self.workflow_registry) {
            let registry = registry.get_mut();
            for workflow in workflows {
                let key = format!(
                    "{}/{}/{}",
                    workflow.document.namespace, workflow.document.name, workflow.document.version
                );
                registry.insert(key, workflow);
            }
        }
    }

    /// Validate workflow graph structure without executing
    ///
    /// This is a static method that can be used for validation without creating an engine instance.
    /// Returns the workflow graph and task name mappings if validation succeeds.
    /// # Errors
    /// Returns an error if the workflow graph cannot be built due to structural issues.
    pub fn validate_workflow_graph(
        workflow: &WorkflowDefinition,
    ) -> Result<(
        DiGraph<(String, TaskDefinition), ()>,
        HashMap<String, NodeIndex>,
    )> {
        graph::build_graph(workflow)
    }

    #[allow(dead_code)]
    /// Register a workflow for nested execution
    ///
    /// # Errors
    /// This function currently does not return errors, but returns `Result` for future extensibility
    pub async fn register_workflow(&self, workflow: WorkflowDefinition) -> Result<()> {
        let key = format!(
            "{}/{}/{}",
            workflow.document.namespace, workflow.document.name, workflow.document.version
        );

        let mut registry = self.workflow_registry.write().await;
        registry.insert(key, workflow);
        Ok(())
    }

    #[allow(dead_code)]
    /// Wait for a workflow instance to complete
    ///
    /// # Errors
    /// Returns an error if the workflow execution fails, times out, or if there are issues retrieving events
    pub async fn wait_for_completion(
        &self,
        instance_id: &str,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value> {
        let start = std::time::Instant::now();

        loop {
            // Check if workflow has completed by looking for WorkflowCompleted event
            let events = self.persistence.get_events(instance_id).await?;

            for event in events.iter().rev() {
                match event {
                    WorkflowEvent::WorkflowCompleted { final_data, .. } => {
                        return Ok(final_data.clone());
                    }
                    WorkflowEvent::WorkflowFailed { error, .. } => {
                        return Err(Error::WorkflowExecution {
                            message: format!("Workflow failed: {error}"),
                        });
                    }
                    WorkflowEvent::WorkflowStarted { .. }
                    | WorkflowEvent::TaskCreated { .. }
                    | WorkflowEvent::TaskStarted { .. }
                    | WorkflowEvent::TaskRetried { .. }
                    | WorkflowEvent::TaskEntered { .. }
                    | WorkflowEvent::TaskCompleted { .. }
                    | WorkflowEvent::WorkflowCorrelationStarted { .. }
                    | WorkflowEvent::WorkflowCorrelationCompleted { .. }
                    | WorkflowEvent::WorkflowCancelled { .. }
                    | WorkflowEvent::WorkflowSuspended { .. }
                    | WorkflowEvent::WorkflowResumed { .. }
                    | WorkflowEvent::TaskCancelled { .. }
                    | WorkflowEvent::TaskSuspended { .. }
                    | WorkflowEvent::TaskResumed { .. }
                    | WorkflowEvent::TaskFaulted { .. }
                    | WorkflowEvent::WorkflowContinuedAsNew { .. }
                    | WorkflowEvent::EventEmitted { .. } => {}
                }
            }

            // Check timeout
            if start.elapsed() > timeout {
                return Err(Error::WorkflowExecution {
                    message: format!("Workflow execution timed out after {timeout:?}"),
                });
            }

            // Wait a bit before checking again
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    #[allow(dead_code)]
    /// Resume a workflow execution from a previously saved checkpoint
    ///
    /// # Errors
    /// Returns an error if the workflow execution fails or if the instance cannot be resumed
    pub async fn resume(
        &self,
        workflow: WorkflowDefinition,
        instance_id: String,
    ) -> Result<serde_json::Value> {
        // Format workflow resume (we'll determine the from_task later in run_instance)
        output::format_workflow_resume(&instance_id, None);
        self.run_instance(workflow, Some(instance_id), serde_json::json!({}))
            .await
    }

    /// Execute a workflow with streaming events
    ///
    /// Returns a handle for observing execution and controlling the workflow.
    /// This is the primary method for library users to execute workflows.
    ///
    /// # Examples
    ///
    /// ```text
    /// # use jackdaw::builder::DurableEngineBuilder;
    /// # use crate::dsl::workflow::WorkflowDefinition;
    /// # use std::time::Duration;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let engine = DurableEngineBuilder::new().build()?;
    ///
    /// // Load workflow from YAML
    /// let workflow_yaml = r#"
    /// document:
    ///   dsl: '1.0.2'
    ///   namespace: example
    ///   name: hello
    ///   version: 1.0.0
    /// do:
    ///   - greet:
    ///       set:
    ///         message: "Hello!"
    /// "#;
    /// let workflow: WorkflowDefinition = serde_saphyr::from_str(workflow_yaml)?;
    ///
    /// let mut handle = engine.execute(workflow, serde_json::json!({})).await?;
    ///
    /// // Option 1: Stream events
    /// while let Some(event) = handle.next_event().await {
    ///     println!("{:?}", event);
    /// }
    ///
    /// // Option 2: Just wait for result
    /// // let result = handle.wait_for_completion(Duration::from_secs(60)).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    /// Returns an error if the workflow execution fails
    pub async fn execute(
        &self,
        workflow: WorkflowDefinition,
        input: serde_json::Value,
    ) -> Result<ExecutionHandle> {
        self.start(workflow, input, None, false).await
    }

    /// Starts a new instance under `instance_id` (a schedule's deterministic id, for example).
    pub(crate) async fn execute_with_id(
        &self,
        workflow: WorkflowDefinition,
        input: serde_json::Value,
        instance_id: String,
    ) -> Result<ExecutionHandle> {
        self.start(workflow, input, Some(instance_id), false).await
    }

    /// Continues an instance that was interrupted, from its checkpoint: tasks that completed are not run again.
    pub(crate) async fn resume_instance(
        &self,
        workflow: WorkflowDefinition,
        instance_id: String,
    ) -> Result<ExecutionHandle> {
        self.start(workflow, serde_json::json!({}), Some(instance_id), true)
            .await
    }

    async fn start(
        &self,
        workflow: WorkflowDefinition,
        input: serde_json::Value,
        instance_id: Option<String>,
        resuming: bool,
    ) -> Result<ExecutionHandle> {
        // Create channels for event streaming and cancellation
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(self.event_buffer_size);
        let (cancel_tx, _cancel_rx) = tokio::sync::mpsc::channel::<()>(1);

        let instance_id = instance_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Clone what we need for the background task
        let executors = self.executors.clone();
        let persistence = self.persistence.clone();
        let cache = self.cache.clone();
        let workflow_registry = self.workflow_registry.clone();
        let clock = Arc::clone(&self.clock);
        let events = self.events.clone();
        #[cfg(feature = "grpc-listener")]
        let grpc_listeners = self.grpc_listeners.clone();
        #[cfg(feature = "http-listener")]
        let http_listeners = self.http_listeners.clone();

        let instance_id_clone = instance_id.clone();

        // Spawn workflow execution in background (with the caller's subscriber, for the trace)
        let run = crate::api::observe::spawn(async move {
            let opening = if resuming {
                WorkflowEvent::WorkflowResumed {
                    instance_id: instance_id_clone.clone(),
                    timestamp: clock.now(),
                }
            } else {
                WorkflowEvent::WorkflowStarted {
                    instance_id: instance_id_clone.clone(),
                    workflow_id: workflow_key(&workflow),
                    timestamp: clock.now(),
                    initial_data: input.clone(),
                }
            };
            let _ = event_tx.send(opening).await;

            // Create a temporary engine for this execution
            // This is needed because we can't move self into the spawned task
            let temp_engine = match DurableEngine::new(persistence.clone(), cache.clone()) {
                Ok(mut engine) => {
                    engine.executors = executors;
                    engine.workflow_registry = workflow_registry;
                    engine.clock = clock;
                    engine.events = events;
                    #[cfg(feature = "grpc-listener")]
                    {
                        engine.grpc_listeners = grpc_listeners;
                    }
                    #[cfg(feature = "http-listener")]
                    {
                        engine.http_listeners = http_listeners;
                    }
                    engine
                }
                Err(e) => {
                    let _ = event_tx
                        .send(WorkflowEvent::WorkflowFailed {
                            instance_id: instance_id_clone.clone(),
                            error: format!("Failed to create engine: {}", e),
                            timestamp: clock.now(),
                        })
                        .await;
                    return;
                }
            };

            // Execute the workflow
            let start_time = Utc::now();
            let result = temp_engine
                .run_instance(
                    workflow.clone(),
                    Some(instance_id_clone.clone()),
                    input.clone(),
                )
                .await;

            // Send completion or failure event
            match result {
                Ok(final_data) => {
                    let duration_ms = (Utc::now() - start_time).num_milliseconds();
                    let _ = event_tx
                        .send(WorkflowEvent::WorkflowCompleted {
                            instance_id: instance_id_clone,
                            final_data,
                            timestamp: temp_engine.clock.now(),
                            duration_ms,
                        })
                        .await;
                }
                Err(e) => {
                    // For TaskExecution errors (e.g., from Raise tasks), extract just the message
                    // to preserve the JSON error format. For other errors, use the full error string.
                    let error_msg = match &e {
                        Error::TaskExecution { message } => message.clone(),
                        Error::WorkflowExecution { .. }
                        | Error::Listener { .. }
                        | Error::Configuration { .. }
                        | Error::Timeout { .. }
                        | Error::Io { .. }
                        | Error::Executor { .. }
                        | Error::Persistence { .. }
                        | Error::Cache { .. }
                        | Error::Context { .. }
                        | Error::Expression { .. }
                        | Error::Serialization { .. }
                        | Error::ListenerSetup { .. } => e.to_string(),
                        #[cfg(feature = "grpc-listener")]
                        Error::Protobuf { .. } | Error::ProtobufDescriptor { .. } => e.to_string(),
                    };
                    let failed = WorkflowEvent::WorkflowFailed {
                        instance_id: instance_id_clone,
                        error: error_msg,
                        timestamp: temp_engine.clock.now(),
                    };
                    // Stored, so recovery knows the instance ended and does not resume it.
                    let _ = temp_engine.persistence.save_event(failed.clone()).await;
                    let _ = event_tx.send(failed).await;
                }
            }

            // TODO: Handle cancellation via cancel_rx
            // TODO: Emit correlation events for perpetual workflows
        });

        Ok(ExecutionHandle::new(instance_id, event_rx, cancel_tx).owning(run.abort_handle()))
    }

    pub(crate) async fn run_instance(
        &self,
        workflow: WorkflowDefinition,
        instance_id: Option<String>,
        initial_data: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Check if workflow has a timeout
        let workflow_timeout = workflow
            .timeout
            .as_ref()
            .and_then(|timeout_def| timeout::parse_timeout_duration(timeout_def).ok());

        // Execute workflow with timeout if specified
        let execution_future = self.run_instance_inner(workflow, instance_id, initial_data);

        if let Some(timeout_duration) = workflow_timeout {
            match tokio::time::timeout(timeout_duration, execution_future).await {
                Ok(result) => result,
                Err(_) => Err(Error::Timeout {
                    message: format!("Workflow execution timed out after {:?}", timeout_duration),
                }),
            }
        } else {
            execution_future.await
        }
    }

    #[async_recursion]
    async fn run_instance_inner(
        &self,
        workflow: WorkflowDefinition,
        instance_id: Option<String>,
        initial_data: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let ctx = Context::new(
            &workflow,
            self.persistence.clone(),
            self.cache.clone(),
            instance_id,
            initial_data,
            Arc::clone(&self.clock),
        )
        .await?;

        let (graph, task_names) = graph::build_graph(&workflow)?;

        // Initialize all listeners BEFORE starting task execution
        self.initialize_listeners(&workflow).await?;

        let current_task_name = ctx.state.current_task.read().await.clone();
        let mut current =
            task_names
                .get(&current_task_name)
                .copied()
                .ok_or(Error::TaskExecution {
                    message: format!("Task not found: {current_task_name}"),
                })?;

        loop {
            let (task_name, task) = &graph[current];

            if let Some(_replayed_result) = ctx.services.history.is_task_completed(task_name) {
                output::format_task_skipped(task_name);
                if let Some(next) = graph.neighbors(current).next() {
                    current = next;
                    continue;
                }
                break;
            }

            ctx.services
                .persistence
                .save_event(WorkflowEvent::TaskEntered {
                    instance_id: ctx.metadata.instance_id.clone(),
                    task_name: task_name.clone(),
                    timestamp: ctx.services.clock.now(),
                })
                .await?;

            // Save the original context before exec_task (which may apply input.from filtering)
            let original_context = ctx.state.data.read().await.clone();

            let result = match self.exec_task(task_name, task, &ctx).await {
                Ok(r) => r,
                Err(e) => {
                    // Task execution failed - emit task.faulted.v1 event
                    let _ = ctx
                        .services
                        .persistence
                        .save_event(WorkflowEvent::TaskFaulted {
                            instance_id: ctx.metadata.instance_id.clone(),
                            task_name: task_name.clone(),
                            error: e.to_string(),
                            timestamp: ctx.services.clock.now(),
                        })
                        .await;

                    return Err(e);
                }
            };

            // Restore the original context before applying export
            // This ensures that input.from filtering doesn't affect export merging
            *ctx.state.data.write().await = original_context;

            // Calculate task duration
            let task_end_time = ctx.services.clock.now();

            // Find the TaskStarted event to calculate duration
            let events = ctx
                .services
                .persistence
                .get_events(&ctx.metadata.instance_id)
                .await?;
            let task_start_time = events
                .iter()
                .rev() // Search in reverse to find most recent TaskStarted
                .find_map(|event| {
                    if let WorkflowEvent::TaskStarted {
                        task_name: name,
                        timestamp,
                        ..
                    } = event
                        && name == task_name
                    {
                        return Some(*timestamp);
                    }
                    None
                })
                .unwrap_or(task_end_time); // Fallback if not found (shouldn't happen)

            let duration_ms = (task_end_time - task_start_time).num_milliseconds();

            // Format task output with duration
            output::format_task_output(&result, duration_ms);

            ctx.services
                .persistence
                .save_event(WorkflowEvent::TaskCompleted {
                    instance_id: ctx.metadata.instance_id.clone(),
                    task_name: task_name.clone(),
                    result: result.clone(),
                    timestamp: task_end_time,
                    duration_ms,
                })
                .await?;

            // Update task_input for the next task before result gets moved
            // According to the spec, each task's transformed output becomes the next task's input
            *ctx.state.task_input.write().await = result.clone();

            // Handle export.as to update context
            export::apply_export_to_context(task, &result, &ctx).await?;

            ctx.save_checkpoint(task_name).await?;

            // Check if we've reached the end of the graph naturally
            let has_next_edge = graph.neighbors(current).next().is_some();

            // Check if the task set a specific next task (e.g., for Switch tasks)
            let next_task_name = {
                let mut next = ctx.state.next_task.write().await;
                next.take() // Take the value and reset to None
            };

            if let Some(next_name) = next_task_name {
                // Task explicitly set the next task (e.g., Switch task)

                // Special cases for flow directives:
                // - "end" means gracefully terminate the workflow
                // - "exit" means exit current scope (terminates workflow since we're in main scope)
                // Note: In future, "exit" will only exit the current scope for nested workflows
                if next_name == "end" || next_name == "exit" {
                    break;
                }

                current = *task_names.get(&next_name).ok_or(Error::TaskExecution {
                    message: format!("Next task not found: {next_name}"),
                })?;
            } else if has_next_edge {
                current = graph
                    .neighbors(current)
                    .next()
                    .ok_or(Error::TaskExecution {
                        message: format!("Next task edge missing after task: {task_name}"),
                    })?;
            } else {
                break;
            }
        }

        // Workflow completed - according to the spec, the workflow output is the last task's transformed output
        // "If no more tasks are defined, the transformed output is passed to the workflow output transformation step."
        // "Workflow `output.as` | Last task's transformed output | Transformed workflow output"
        let mut final_data = ctx.state.task_input.read().await.clone();

        // Apply workflow output filter if specified
        if let Some(output_config) = &workflow.output
            && let Some(as_expr) = &output_config.as_
        {
            // Handle both string and object forms of the expression
            let expr_str = if let Some(s) = as_expr.as_str() {
                s
            } else {
                // If it's not a string, convert it to JSON string
                // This handles complex runtime expressions that are objects
                &serde_json::to_string(as_expr)?
            };

            // Output filtering can use either:
            // 1. Wrapped expressions: ${ .field } (newer spec examples)
            // 2. Bare JQ expressions: .field (older examples)
            final_data = if expr_str.trim().starts_with("${") {
                // Wrapped expression - use evaluate_expression which handles ${ } syntax
                crate::expressions::evaluate_expression(expr_str, &final_data)?
            } else {
                // Bare JQ expression - use evaluate_jq directly
                crate::expressions::evaluate_jq(expr_str, &final_data)?
            };
        } // Remove internal metadata fields from final output
        if let serde_json::Value::Object(ref mut obj) = final_data {
            obj.remove("__workflow");
            obj.remove("__runtime");
        }

        // Calculate workflow duration
        let workflow_end_time = ctx.services.clock.now();
        let events = ctx
            .services
            .persistence
            .get_events(&ctx.metadata.instance_id)
            .await?;
        let workflow_start_time = events
            .iter()
            .find_map(|event| {
                if let WorkflowEvent::WorkflowStarted { timestamp, .. } = event {
                    Some(*timestamp)
                } else {
                    None
                }
            })
            .unwrap_or(workflow_end_time);

        let workflow_duration_ms = (workflow_end_time - workflow_start_time).num_milliseconds();

        ctx.services
            .persistence
            .save_event(WorkflowEvent::WorkflowCompleted {
                instance_id: ctx.metadata.instance_id.clone(),
                final_data: final_data.clone(),
                timestamp: workflow_end_time,
                duration_ms: workflow_duration_ms,
            })
            .await?;

        // Format workflow completion with output and duration
        output::format_workflow_output(&final_data, workflow_duration_ms);

        Ok(final_data)
    }
}

/// The key YAML workflows are registered under, and stored `WorkflowStarted` events name: `namespace/name/version`.
pub(crate) fn workflow_key(workflow: &WorkflowDefinition) -> String {
    format!(
        "{}/{}/{}",
        workflow.document.namespace, workflow.document.name, workflow.document.version
    )
}

/// The event an [`EventEmitted`](WorkflowEvent::EventEmitted) record holds, if it was published at `since` or later.
fn emitted_after(
    stored: WorkflowEvent,
    since: chrono::DateTime<chrono::Utc>,
) -> Option<serde_json::Value> {
    match stored {
        WorkflowEvent::EventEmitted {
            event, timestamp, ..
        } if timestamp >= since => Some(event),
        WorkflowEvent::EventEmitted { .. }
        | WorkflowEvent::WorkflowStarted { .. }
        | WorkflowEvent::TaskEntered { .. }
        | WorkflowEvent::TaskCreated { .. }
        | WorkflowEvent::TaskStarted { .. }
        | WorkflowEvent::TaskRetried { .. }
        | WorkflowEvent::TaskCompleted { .. }
        | WorkflowEvent::WorkflowCompleted { .. }
        | WorkflowEvent::WorkflowCorrelationStarted { .. }
        | WorkflowEvent::WorkflowCorrelationCompleted { .. }
        | WorkflowEvent::WorkflowFailed { .. }
        | WorkflowEvent::WorkflowCancelled { .. }
        | WorkflowEvent::WorkflowSuspended { .. }
        | WorkflowEvent::WorkflowResumed { .. }
        | WorkflowEvent::TaskCancelled { .. }
        | WorkflowEvent::TaskSuspended { .. }
        | WorkflowEvent::TaskResumed { .. }
        | WorkflowEvent::TaskFaulted { .. }
        | WorkflowEvent::WorkflowContinuedAsNew { .. } => None,
    }
}

fn lock_pruned(
    pruned: &std::sync::Mutex<Option<chrono::NaiveDate>>,
) -> std::sync::MutexGuard<'_, Option<chrono::NaiveDate>> {
    pruned
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
