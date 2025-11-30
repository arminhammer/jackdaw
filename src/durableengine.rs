use async_recursion::async_recursion;
use chrono::Utc;
use petgraph::{graph::DiGraph, stable_graph::NodeIndex};
use serverless_workflow_core::models::task::TaskDefinition;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::{
    context::Context,
    executor::Executor,
    listeners::grpc::GrpcListener,
    output,
    persistence::PersistenceProvider,
    providers::{
        executors::{OpenApiExecutor, PythonExecutor, RestExecutor},
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
    /// Create a new DurableEngine instance
    ///
    /// # Errors
    /// Currently, this function does not return errors, but returns `Result` for future extensibility
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

    /// Validate workflow graph structure without executing
    ///
    /// This is a static method that can be used for validation without creating an engine instance.
    /// Returns the workflow graph and task name mappings if validation succeeds.
    pub fn validate_workflow_graph(
        workflow: &WorkflowDefinition,
    ) -> Result<(
        DiGraph<(String, TaskDefinition), ()>,
        HashMap<String, NodeIndex>,
    )> {
        graph::build_graph(workflow)
    }

    #[allow(dead_code)]
    /// Start a workflow execution with empty initial data
    ///
    /// # Errors
    /// Returns an error if the workflow execution fails or if there are issues with persistence
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
                    message: format!("Task not found: {current_task_name}"),
                })?;

        loop {
            let (task_name, task) = &graph[current];

            if let Some(_replayed_result) = ctx.history.is_task_completed(task_name) {
                output::format_task_skipped(task_name);
                if let Some(next) = graph.neighbors(current).next() {
                    current = next;
                    continue;
                }
                break;
            }

            ctx.persistence
                .save_event(WorkflowEvent::TaskEntered {
                    instance_id: ctx.instance_id.clone(),
                    task_name: task_name.clone(),
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
                    task_name: task_name.clone(),
                    result: result.clone(),
                    timestamp: Utc::now(),
                })
                .await?;

            // Update task_input for the next task before result gets moved
            // According to the spec, each task's transformed output becomes the next task's input
            *ctx.task_input.write().await = result.clone();

            // Handle export.as to update context
            // According to spec: "defaults to the expression that returns the existing context"
            // So if no export.as is specified, context remains unchanged
            let export_config = match task {
                TaskDefinition::Call(t) => t.common.export.as_ref(),
                TaskDefinition::Do(t) => t.common.export.as_ref(),
                TaskDefinition::Emit(t) => t.common.export.as_ref(),
                TaskDefinition::For(t) => t.common.export.as_ref(),
                TaskDefinition::Fork(t) => t.common.export.as_ref(),
                TaskDefinition::Listen(t) => t.common.export.as_ref(),
                TaskDefinition::Raise(t) => t.common.export.as_ref(),
                TaskDefinition::Run(t) => t.common.export.as_ref(),
                TaskDefinition::Set(t) => t.common.export.as_ref(),
                TaskDefinition::Switch(t) => t.common.export.as_ref(),
                TaskDefinition::Try(t) => t.common.export.as_ref(),
                TaskDefinition::Wait(t) => t.common.export.as_ref(),
            };

            if let Some(export_def) = export_config {
                if let Some(export_expr) = &export_def.as_
                    && let Some(expr_str) = export_expr.as_str()
                {
                    // Evaluate export.as expression on the transformed task output
                    // The result becomes the new context
                    let new_context = crate::expressions::evaluate_expression(expr_str, &result)?;
                    *ctx.data.write().await = new_context;
                }
            } else {
                // No explicit export.as - apply default behavior
                // Default: merge the transformed task output into the existing context
                // This respects the task's output - every task produces output that should be used
                let mut current_context = ctx.data.write().await;
                if let serde_json::Value::Object(result_obj) = &result {
                    if let Some(context_obj) = (*current_context).as_object_mut() {
                        for (key, value) in result_obj {
                            context_obj.insert(key.clone(), value.clone());
                        }
                    }
                } else {
                    // If result is not an object, we cannot merge - replace the context entirely
                    *current_context = result.clone();
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
                    message: format!("Next task not found: {next_name}"),
                })?;
            } else if has_next_edge {
                current = graph.neighbors(current).next().unwrap();
            } else {
                break;
            }
        }

        // Workflow completed - according to the spec, the workflow output is the last task's transformed output
        // "If no more tasks are defined, the transformed output is passed to the workflow output transformation step."
        // "Workflow `output.as` | Last task's transformed output | Transformed workflow output"
        let mut final_data = ctx.task_input.read().await.clone();

        // Apply workflow output filter if specified
        if let Some(output_config) = &workflow.output {
            if let Some(as_expr) = &output_config.as_
                && let Some(expr_str) = as_expr.as_str()
            {
                final_data = crate::expressions::evaluate_expression(expr_str, &final_data)?;
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
                    message: format!("Unknown visualization tool: {tool}"),
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
