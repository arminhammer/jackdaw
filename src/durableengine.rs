use async_recursion::async_recursion;
use chrono::Utc;
use petgraph::{graph::DiGraph, stable_graph::NodeIndex};
use serverless_workflow_core::models::task::{ListenTaskDefinition, TaskDefinition};
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::{
    cache::CacheEntry,
    cache::compute_cache_key,
    context::Context,
    executor::Executor,
    listeners::{EventSource, Listener, grpc::GrpcListener, http::HttpListener},
    output,
    persistence::PersistenceProvider,
    providers::{
        executors::{OpenApiExecutor, PythonExecutor, RestExecutor, TypeScriptExecutor},
        visualization::{D2Provider, ExecutionState, GraphvizProvider, VisualizationProvider},
    },
    workflow::WorkflowEvent,
};

use super::cache::CacheProvider;

// Submodules
mod catalog;
mod graph;
mod listeners;
mod tasks;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Workflow execution error: {message}"))]
    WorkflowExecution { message: String },

    #[snafu(display("Task execution error: {message}"))]
    TaskExecution { message: String },

    #[snafu(display("Listener error: {message}"))]
    Listener { message: String },

    #[snafu(display("Configuration error: {message}"))]
    Configuration { message: String },

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

    #[snafu(display("Protobuf compilation error: {source}"))]
    Protobuf { source: protox::Error },

    #[snafu(display("Protobuf descriptor error: {source}"))]
    ProtobufDescriptor {
        source: prost_reflect::DescriptorError,
    },

    #[snafu(display("Visualization error: {source}"))]
    Visualization {
        source: crate::providers::visualization::Error,
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

impl From<protox::Error> for Error {
    fn from(source: protox::Error) -> Self {
        Error::Protobuf { source }
    }
}

impl From<prost_reflect::DescriptorError> for Error {
    fn from(source: prost_reflect::DescriptorError) -> Self {
        Error::ProtobufDescriptor { source }
    }
}

impl From<crate::providers::visualization::Error> for Error {
    fn from(source: crate::providers::visualization::Error) -> Self {
        Error::Visualization { source }
    }
}

pub struct DurableEngine {
    executors: HashMap<String, Box<dyn Executor>>,
    persistence: Arc<dyn PersistenceProvider>,
    cache: Arc<dyn CacheProvider>,
    /// Registry of active gRPC listeners, keyed by bind address
    /// Using Arc<GrpcListener> to allow adding methods progressively
    grpc_listeners: Arc<RwLock<HashMap<String, Arc<GrpcListener>>>>,
    /// Registry of active HTTP listeners, keyed by bind address
    /// Using Arc<HttpListener> to allow adding routes progressively
    http_listeners: Arc<RwLock<HashMap<String, Arc<crate::listeners::HttpListener>>>>,
    /// Registry of workflows for nested execution, keyed by "namespace/name/version"
    workflow_registry: Arc<RwLock<HashMap<String, WorkflowDefinition>>>,
}

impl std::fmt::Debug for DurableEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableEngine")
            .field("executors", &self.executors.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl DurableEngine {
    pub fn new(
        persistence: Arc<dyn PersistenceProvider>,
        cache: Arc<dyn CacheProvider>,
    ) -> Result<Self> {
        let mut executors: HashMap<String, Box<dyn Executor>> = HashMap::new();
        executors.insert(
            "http".into(),
            Box::new(RestExecutor(reqwest::Client::new())),
        );
        executors.insert(
            "rest".into(),
            Box::new(RestExecutor(reqwest::Client::new())),
        );
        executors.insert(
            "openapi".into(),
            Box::new(OpenApiExecutor(reqwest::Client::new())),
        );
        executors.insert("python".into(), Box::new(PythonExecutor::new()));
        Ok(Self {
            executors,
            persistence,
            cache,
            grpc_listeners: Arc::new(RwLock::new(HashMap::new())),
            http_listeners: Arc::new(RwLock::new(HashMap::new())),
            workflow_registry: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn start(&self, workflow: WorkflowDefinition) -> Result<String> {
        let (instance_id, _) = self
            .start_with_input(workflow, serde_json::json!({}))
            .await?;
        Ok(instance_id)
    }

    #[async_recursion(?Send)]
    pub async fn start_with_input(
        &self,
        workflow: WorkflowDefinition,
        initial_data: serde_json::Value,
    ) -> Result<(String, serde_json::Value)> {
        let instance_id = uuid::Uuid::new_v4().to_string();

        // Format workflow start
        output::format_workflow_start(&workflow.document.name, &instance_id);

        // Show initial input if not empty
        if !initial_data.is_null() && initial_data != serde_json::json!({}) {
            output::format_workflow_input(&initial_data);
        }

        match self
            .run_instance(workflow, Some(instance_id.clone()), initial_data)
            .await
        {
            Ok(final_data) => Ok((instance_id, final_data)),
            Err(e) => {
                // Save WorkflowFailed event before returning error
                let _ = self
                    .persistence
                    .save_event(WorkflowEvent::WorkflowFailed {
                        instance_id: instance_id.clone(),
                        error: e.to_string(),
                        timestamp: Utc::now(),
                    })
                    .await;
                Err(e)
            }
        }
    }

    pub async fn register_workflow(&self, workflow: WorkflowDefinition) -> Result<()> {
        let key = format!(
            "{}/{}/{}",
            workflow.document.namespace, workflow.document.name, workflow.document.version
        );

        let mut registry = self.workflow_registry.write().await;
        registry.insert(key, workflow);
        Ok(())
    }

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
                            message: format!("Workflow failed: {}", error),
                        });
                    }
                    _ => {}
                }
            }

            // Check timeout
            if start.elapsed() > timeout {
                return Err(Error::WorkflowExecution {
                    message: format!("Workflow execution timed out after {:?}", timeout),
                });
            }

            // Wait a bit before checking again
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

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

    async fn run_instance(
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
        )
        .await?;

        let (graph, task_names) = graph::build_graph(&workflow)?;

        // Initialize all listeners BEFORE starting task execution
        self.initialize_listeners(&workflow).await?;

        let current_task_name = ctx.current_task.read().await.clone();
        let mut current =
            task_names
                .get(&current_task_name)
                .copied()
                .ok_or(Error::TaskExecution {
                    message: format!("Task not found: {}", current_task_name),
                })?;

        loop {
            let (task_name, task) = &graph[current];

            if let Some(_replayed_result) = ctx.history.is_task_completed(task_name) {
                output::format_task_skipped(task_name);
                if let Some(next) = graph.neighbors(current).next() {
                    current = next;
                    continue;
                } else {
                    break;
                }
            }

            ctx.persistence
                .save_event(WorkflowEvent::TaskEntered {
                    instance_id: ctx.instance_id.clone(),
                    task_name: task_name.to_string(),
                    timestamp: Utc::now(),
                })
                .await?;

            let result = self.exec_task(task_name, task, &ctx).await?;

            // Format task output
            output::format_task_output(&result);
            output::format_task_complete(task_name);

            ctx.persistence
                .save_event(WorkflowEvent::TaskCompleted {
                    instance_id: ctx.instance_id.clone(),
                    task_name: task_name.to_string(),
                    result: result.clone(),
                    timestamp: Utc::now(),
                })
                .await?;

            // Only merge task results for tasks that don't directly modify context
            // Set, Do, For, Switch, and Emit tasks modify context directly or produce standalone output
            // Call tasks with output filters also modify context directly
            match task {
                TaskDefinition::Set(_)
                | TaskDefinition::Do(_)
                | TaskDefinition::For(_)
                | TaskDefinition::Switch(_)
                | TaskDefinition::Emit(_) => {
                    // These tasks already modified the context directly or produce output that shouldn't be nested
                }
                TaskDefinition::Call(call_task) => {
                    // Check if there's an output filter - if so, result is already merged directly
                    let has_output_filter = call_task
                        .common
                        .output
                        .as_ref()
                        .and_then(|o| o.as_.as_ref())
                        .and_then(|v| v.as_str())
                        .is_some();

                    if !has_output_filter {
                        // For call tasks without output filters, merge the result directly (not nested)
                        // This makes call task results appear at the root level of workflow output
                        if let serde_json::Value::Object(map) = &result {
                            for (key, value) in map {
                                ctx.merge(key, value.clone()).await;
                            }
                        } else {
                            // If result is not an object, store with task name
                            ctx.merge(task_name, result).await;
                        }
                    }
                }
                TaskDefinition::Run(run_task) => {
                    // For run tasks that execute workflows, merge the result directly (not nested)
                    // This makes nested workflow results appear at the root level like call tasks
                    if run_task.run.workflow.is_some() {
                        if let serde_json::Value::Object(map) = &result {
                            for (key, value) in map {
                                ctx.merge(key, value.clone()).await;
                            }
                        } else {
                            // If result is not an object, store with task name
                            ctx.merge(task_name, result).await;
                        }
                    } else {
                        // For other run tasks (script, container, shell), keep nested under task name
                        ctx.merge(task_name, result).await;
                    }
                }
                _ => {
                    // Other tasks (Fork, etc.) should merge their results with task name
                    ctx.merge(task_name, result).await;
                }
            }
            ctx.save_checkpoint(task_name).await?;

            // Check if we've reached the end of the graph naturally
            let has_next_edge = graph.neighbors(current).next().is_some();

            // Check if the task set a specific next task (e.g., for Switch tasks)
            let next_task_name = {
                let mut next = ctx.next_task.write().await;
                next.take() // Take the value and reset to None
            };

            if let Some(next_name) = next_task_name {
                // Task explicitly set the next task (e.g., Switch task)

                // Special case: "end" means terminate the workflow
                if next_name == "end" {
                    break;
                }

                current = *task_names.get(&next_name).ok_or(Error::TaskExecution {
                    message: format!("Next task not found: {}", next_name),
                })?;
            } else if has_next_edge {
                current = graph.neighbors(current).next().unwrap();
            } else {
                break;
            }
        }

        // Workflow completed - clean up final data
        let mut final_data = ctx.data.read().await.clone();

        // Remove initial input fields from final output if data was modified by tasks
        // However, keep any keys that were explicitly set by task outputs
        let data_was_modified = *ctx.data_modified.read().await;
        if data_was_modified {
            if let Some(obj) = final_data.as_object_mut() {
                eprintln!("DEBUG: Before cleanup - keys: {:?}", obj.keys().collect::<Vec<_>>());

                // Remove internal metadata fields before checking length
                obj.remove("__workflow");
                obj.remove("__runtime");

                if let Some(input_obj) = ctx.initial_input.as_object() {
                    let task_output_keys = ctx.task_output_keys.read().await;
                    eprintln!("DEBUG: Input keys: {:?}, Task output keys: {:?}",
                        input_obj.keys().collect::<Vec<_>>(),
                        task_output_keys.iter().collect::<Vec<_>>());
                    for key in input_obj.keys() {
                        // Only remove input keys that weren't set by tasks
                        if !task_output_keys.contains(key) {
                            eprintln!("DEBUG: Removing input key: {}", key);
                            obj.remove(key);
                        }
                    }
                }

                eprintln!("DEBUG: After cleanup - keys: {:?}, length: {}",
                    obj.keys().collect::<Vec<_>>(), obj.len());

                // If there's only one task output key that's a scalar from output filtering, unwrap it
                if obj.len() == 1 {
                    if let Some((key, value)) = obj.iter().next() {
                        let scalar_tasks = ctx.scalar_output_tasks.read().await;
                        // Unwrap if it's a scalar value from output filtering
                        if scalar_tasks.contains(key) && !value.is_object() && !value.is_array() {
                            eprintln!("DEBUG: Unwrapping scalar task output: {} = {:?}", key, value);
                            final_data = value.clone();
                        }
                    }
                }
            }
        }

        // Apply workflow output filter if specified
        if let Some(output_config) = &workflow.output {
            if let Some(as_expr) = &output_config.as_ {
                if let Some(expr_str) = as_expr.as_str() {
                    final_data = crate::expressions::evaluate_expression(expr_str, &final_data)?;
                }
            }
        }

        // Remove internal metadata fields from final output
        if let serde_json::Value::Object(ref mut obj) = final_data {
            obj.remove("__workflow");
            obj.remove("__runtime");
        }

        ctx.persistence
            .save_event(WorkflowEvent::WorkflowCompleted {
                instance_id: ctx.instance_id.clone(),
                final_data: final_data.clone(),
                timestamp: Utc::now(),
            })
            .await?;

        // Format workflow completion with output
        output::format_workflow_output(&final_data);

        Ok(final_data)
    }
}

impl DurableEngine {
    /// Visualize workflow execution after completion
    ///
    /// # Arguments
    /// * `workflow` - The workflow definition
    /// * `instance_id` - The workflow instance to visualize
    /// * `output_path` - Optional output path (None for stdout/ASCII)
    /// * `format` - Output format
    /// * `tool` - Visualization tool to use ("graphviz" or "d2")
    pub async fn visualize_execution(
        &self,
        workflow: &WorkflowDefinition,
        instance_id: &str,
        output_path: Option<&std::path::Path>,
        format: crate::providers::visualization::DiagramFormat,
        tool: &str,
    ) -> Result<()> {
        // Get execution events
        let _events = self.persistence.get_events(instance_id).await?;

        // Build execution state from events
        let execution_state = ExecutionState::new();

        // Select provider
        let provider: Box<dyn VisualizationProvider> = match tool {
            "graphviz" => Box::new(GraphvizProvider::new()),
            "d2" => Box::new(D2Provider::new()),
            _ => {
                return Err(Error::Configuration {
                    message: format!("Unknown visualization tool: {}", tool),
                });
            }
        };

        // Check availability
        if !provider.is_available()? {
            return Err(Error::Configuration {
                message: format!("{} is not installed or not available", provider.name()),
            });
        }

        // Render diagram with execution state
        provider.render(workflow, output_path, format, Some(&execution_state))?;

        Ok(())
    }
}
