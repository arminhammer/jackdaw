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
    listeners::{
        grpc::GrpcListener,
        http::HttpListener,
        EventSource,
        Listener,
    },
    output,
    persistence::PersistenceProvider,
    providers::{
        executors::{OpenApiExecutor, PythonExecutor, RestExecutor, TypeScriptExecutor},
        visualization::{D2Provider, ExecutionState, GraphvizProvider, VisualizationProvider},
    },
    workflow::WorkflowEvent,
};

use super::cache::CacheProvider;

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
    ProtobufDescriptor { source: prost_reflect::DescriptorError },

    #[snafu(display("Visualization error: {source}"))]
    Visualization { source: crate::providers::visualization::Error },
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
        self.start_with_input(workflow, serde_json::json!({})).await
    }

    #[async_recursion(?Send)]
    pub async fn start_with_input(
        &self,
        workflow: WorkflowDefinition,
        initial_data: serde_json::Value,
    ) -> Result<String> {
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
            Ok(_) => Ok(instance_id),
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
                        return Err(Error::WorkflowExecution { message: format!("Workflow failed: {}", error) });
                    }
                    _ => {}
                }
            }

            // Check timeout
            if start.elapsed() > timeout {
                return Err(Error::WorkflowExecution { message: format!("Workflow execution timed out after {:?}", timeout) });
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

        let (graph, task_names) = self.build_graph(&workflow)?;

        // Initialize all listeners BEFORE starting task execution
        self.initialize_listeners(&workflow).await?;

        let current_task_name = ctx.current_task.read().await.clone();
        let mut current = task_names
            .get(&current_task_name)
            .copied()
            .ok_or(Error::TaskExecution { message: format!("Task not found: {}", current_task_name) })?;

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

                current = *task_names
                    .get(&next_name)
                    .ok_or(Error::TaskExecution { message: format!("Next task not found: {}", next_name) })?;
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
                if let Some(input_obj) = ctx.initial_input.as_object() {
                    let task_output_keys = ctx.task_output_keys.read().await;
                    for key in input_obj.keys() {
                        // Only remove input keys that weren't set by tasks
                        if !task_output_keys.contains(key) {
                            obj.remove(key);
                        }
                    }
                }

                // If there's only one task output key and it was marked as a scalar output from filtering,
                // unwrap it to return just the scalar (for workflows with single-task scalar outputs)
                if obj.len() == 1 {
                    if let Some((key, value)) = obj.iter().next() {
                        let scalar_tasks = ctx.scalar_output_tasks.read().await;
                        if scalar_tasks.contains(key) && !value.is_object() && !value.is_array() {
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

    fn build_graph(
        &self,
        workflow: &WorkflowDefinition,
    ) -> Result<(
        DiGraph<(String, TaskDefinition), ()>,
        HashMap<String, NodeIndex>,
    )> {
        let mut graph = DiGraph::new();
        let mut nodes = HashMap::new();
        let mut task_names = Vec::new();

        // Iterate over all task entries in the Map and preserve order
        for entry in &workflow.do_.entries {
            for (name, task) in entry {
                let node = graph.add_node((name.clone(), task.clone()));
                nodes.insert(name.clone(), node);
                task_names.push(name.clone());
            }
        }

        // Build explicit edges based on 'then' transitions
        let mut has_explicit_transitions = false;
        for entry in &workflow.do_.entries {
            for (name, task) in entry {
                let src = nodes.get(name).ok_or(Error::TaskExecution { message: "Task not found".to_string() })?;
                let transitions = get_task_transitions(task);
                if !transitions.is_empty() {
                    has_explicit_transitions = true;
                    for target in transitions {
                        if let Some(&dst) = nodes.get(&target) {
                            graph.add_edge(*src, dst, ());
                        }
                    }
                }
            }
        }

        // If no explicit transitions, create implicit sequential edges
        if !has_explicit_transitions && task_names.len() > 1 {
            for i in 0..task_names.len() - 1 {
                let src = nodes.get(&task_names[i]).ok_or(Error::TaskExecution { message: "Task not found".to_string() })?;
                let dst = nodes
                    .get(&task_names[i + 1])
                    .ok_or(Error::TaskExecution { message: "Task not found".to_string() })?;
                graph.add_edge(*src, *dst, ());
            }
        }

        Ok((graph, nodes))
    }

    /// Initialize all listeners from the workflow before task execution begins
    /// This scans the workflow for all Listen tasks, groups them by bind address,
    /// and starts all listeners together with their complete route tables
    async fn initialize_listeners(&self, workflow: &WorkflowDefinition) -> Result<()> {

        // Collect all HTTP routes grouped by (bind_addr, openapi_path)
        // Key: (bind_addr, openapi_path), Value: Vec of (path, task_name, handler)
        let mut http_routes: HashMap<
            (String, String),
            Vec<(
                String,
                String,
                Arc<dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value> + Send + Sync>,
            )>,
        > = HashMap::new();

        // Collect all gRPC methods grouped by (bind_addr, proto_path, service_name)
        // Key: (bind_addr, proto_path, service_name), Value: Vec of (method_name, task_name, handler)
        let mut grpc_methods: HashMap<
            (String, String, String),
            Vec<(
                String,
                String,
                Arc<dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value> + Send + Sync>,
            )>,
        > = HashMap::new();

        // Scan all tasks for Listen tasks
        for entry in &workflow.do_.entries {
            for (task_name, task) in entry {
                if let TaskDefinition::Listen(listen_task) = task {
                    // Extract event source and handler information
                    let (source_value, schema_path_opt) =
                        self.extract_listen_source(listen_task)?;
                    let event_source: EventSource = serde_json::from_value(source_value)?;

                    // Handle HTTP listeners
                    if event_source.uri.starts_with("http://")
                        || event_source.uri.starts_with("https://")
                    {
                        // Parse bind address and path from URI
                        let uri = &event_source.uri;
                        let without_scheme = uri
                            .strip_prefix("http://")
                            .or_else(|| uri.strip_prefix("https://"))
                            .ok_or_else(|| Error::Listener { message: "Invalid HTTP URI".to_string() })?;

                        let parts: Vec<&str> = without_scheme.splitn(2, '/').collect();
                        let mut bind_addr = parts[0].to_string();

                        // Convert localhost to 127.0.0.1 for SocketAddr parsing
                        if bind_addr.starts_with("localhost:") {
                            bind_addr = bind_addr.replace("localhost:", "127.0.0.1:");
                        }

                        let path = if parts.len() > 1 {
                            format!("/{}", parts[1])
                        } else {
                            "/".to_string()
                        };

                        let openapi_path = schema_path_opt
                            .ok_or_else(|| Error::Listener { message: "HTTP listener requires OpenAPI schema".to_string() })?;

                        // Create handler for this route
                        let handler = self.create_handler_from_listen_task(listen_task)?;

                        // Group by (bind_addr, openapi_path) - different specs can coexist on same port
                        http_routes
                            .entry((bind_addr.clone(), openapi_path.clone()))
                            .or_insert_with(Vec::new)
                            .push((path, task_name.clone(), handler));
                    }
                    // Handle gRPC listeners
                    else if event_source.uri.starts_with("grpc://") {
                        // Parse bind address and method from URI (e.g., grpc://localhost:50051/calculator.Calculator/Add)
                        let uri = &event_source.uri;
                        let without_scheme = uri
                            .strip_prefix("grpc://")
                            .ok_or_else(|| Error::Listener { message: "Invalid gRPC URI".to_string() })?;

                        let parts: Vec<&str> = without_scheme.splitn(2, '/').collect();
                        let mut bind_addr = parts[0].to_string();

                        // Convert localhost to 127.0.0.1 for SocketAddr parsing
                        if bind_addr.starts_with("localhost:") {
                            bind_addr = bind_addr.replace("localhost:", "127.0.0.1:");
                        }

                        // Extract service and method from the path (e.g., "calculator.Calculator/Add")
                        let method_path = if parts.len() > 1 {
                            parts[1]
                        } else {
                            return Err(Error::Listener { message: "gRPC URI must include service/method path".to_string() });
                        };

                        let method_parts: Vec<&str> = method_path.split('/').collect();
                        if method_parts.len() != 2 {
                            return Err(Error::Listener { message: "gRPC method path must be in format 'service.Name/Method'".to_string() });
                        }
                        let service_name = method_parts[0].to_string();
                        let method_name = method_parts[1].to_string();

                        let proto_path = schema_path_opt
                            .ok_or_else(|| Error::Listener { message: "gRPC listener requires proto schema".to_string() })?;

                        // Create handler for this method
                        let handler = self.create_handler_from_listen_task(listen_task)?;

                        // Group by (bind_addr, proto_path, service_name)
                        grpc_methods
                            .entry((bind_addr.clone(), proto_path.clone(), service_name.clone()))
                            .or_insert_with(Vec::new)
                            .push((method_name, task_name.clone(), handler));
                    }
                }
            }
        }

        // Now create all HTTP listeners with their complete route tables
        let mut http_listeners = self.http_listeners.write().await;

        for ((bind_addr, openapi_path), routes) in http_routes {
            // Build route handlers map
            let mut route_handlers = std::collections::HashMap::new();
            for (path, task_name, handler) in routes {
                route_handlers.insert(path.clone(), handler);
                println!(
                    "  Registering route {} for task {} on {}",
                    path, task_name, bind_addr
                );
            }

            // Create and start the listener with all routes
            let listener =
                HttpListener::new_multi_route(bind_addr.clone(), &openapi_path, route_handlers)?;
            let listener_arc = Arc::new(listener);
            listener_arc.start().await?;

            // Wait a bit for the server to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            http_listeners.insert(bind_addr.clone(), listener_arc);
            println!("  HTTP listener started on {}", bind_addr);
        }

        // Now create all gRPC listeners with their complete method tables
        let mut grpc_listeners = self.grpc_listeners.write().await;

        for ((bind_addr, proto_path, service_name), methods) in grpc_methods {
            use prost_reflect::DynamicMessage;

            // Compile proto file to get descriptors
            let file_descriptor_set = protox::compile([proto_path.as_str()], ["."])?;
            let pool =
                prost_reflect::DescriptorPool::from_file_descriptor_set(file_descriptor_set)?;

            // Get service descriptor
            let service_descriptor = pool
                .get_service_by_name(&service_name)
                .ok_or_else(|| Error::Listener { message: format!("Service {} not found in proto file", service_name) })?;

            // Build method handlers map - convert JSON handlers to DynamicMessage handlers
            let mut method_handlers: std::collections::HashMap<
                String,
                Arc<dyn Fn(DynamicMessage) -> crate::listeners::Result<DynamicMessage> + Send + Sync>,
            > = std::collections::HashMap::new();

            for (method_name, task_name, json_handler) in methods {
                println!(
                    "  Registering gRPC method {}/{} for task {} on {}",
                    service_name, method_name, task_name, bind_addr
                );

                // Get method descriptor for this method
                let method_descriptor = service_descriptor
                    .methods()
                    .find(|m| m.name() == method_name)
                    .ok_or_else(|| {
                        Error::Listener { message: format!("Method {} not found in service {}", method_name, service_name) }
                    })?;

                let output_descriptor = method_descriptor.output();

                // Clone the Arc so each closure gets its own reference
                let json_handler_clone = json_handler.clone();

                // Create a wrapper that converts DynamicMessage to JSON, calls the JSON handler, then converts back
                let wrapped_handler: Arc<
                    dyn Fn(DynamicMessage) -> crate::listeners::Result<DynamicMessage> + Send + Sync,
                > = Arc::new(
                    move |request_msg: DynamicMessage| -> crate::listeners::Result<DynamicMessage> {
                        // Convert DynamicMessage to JSON
                        let request_json = dynamic_message_to_json(&request_msg)
                            .map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to convert DynamicMessage to JSON: {}", e) })?;

                        // Call the JSON handler
                        let response_json = json_handler_clone(request_json)?;

                        // Convert JSON response back to DynamicMessage using the output descriptor
                        let response_msg =
                            json_to_dynamic_message(&response_json, output_descriptor.clone())
                                .map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to convert JSON to DynamicMessage: {}", e) })?;
                        Ok(response_msg)
                    },
                );

                method_handlers.insert(method_name.clone(), wrapped_handler);
            }

            // Create and start the gRPC listener with all methods
            let listener = GrpcListener::new_multi_method(
                bind_addr.clone(),
                &proto_path,
                &service_name,
                method_handlers,
            )?;
            let listener_arc = Arc::new(listener);
            listener_arc.start().await?;

            // Wait a bit for the server to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            grpc_listeners.insert(bind_addr.clone(), listener_arc);
            println!("  gRPC listener started on {}", bind_addr);
        }

        Ok(())
    }

    /// Extract event source and OpenAPI path from a Listen task
    fn extract_listen_source(
        &self,
        listen_task: &ListenTaskDefinition,
    ) -> Result<(serde_json::Value, Option<String>)> {
        let (_event_filter, with_attrs, source_value) =
            if let Some(one_filter) = &listen_task.listen.to.one {
                let with_attrs = one_filter
                    .with
                    .as_ref()
                    .ok_or_else(|| Error::Configuration { message: "Listen task requires 'with' attributes".to_string() })?;
                let source_value = with_attrs
                    .get("source")
                    .ok_or_else(|| Error::Configuration { message: "Listen task requires 'source' in 'with' attributes".to_string() })?;
                (one_filter, with_attrs, source_value)
            } else if let Some(any_filters) = &listen_task.listen.to.any {
                let first_filter = any_filters.first().ok_or_else(|| {
                    Error::Configuration { message: "Listen task 'any' requires at least one event filter".to_string() }
                })?;
                let with_attrs = first_filter
                    .with
                    .as_ref()
                    .ok_or_else(|| Error::Configuration { message: "Listen task requires 'with' attributes".to_string() })?;
                let source_value = with_attrs
                    .get("source")
                    .ok_or_else(|| Error::Configuration { message: "Listen task requires 'source' in 'with' attributes".to_string() })?;
                (first_filter, with_attrs, source_value)
            } else {
                return Err(Error::Configuration { message: "Listen task requires either 'one' or 'any' event filter".to_string() });
            };

        // Get OpenAPI schema path if present
        let source_obj: EventSource = serde_json::from_value(source_value.clone())?;
        let openapi_path = source_obj
            .schema
            .as_ref()
            .map(|s| s.resource.endpoint.clone());

        Ok((source_value.clone(), openapi_path))
    }

    /// Create a handler function from a Listen task's foreach.do block
    /// Extracts the handler module and function from the first Call task in the foreach block
    fn create_handler_from_listen_task(
        &self,
        listen_task: &ListenTaskDefinition,
    ) -> Result<Arc<dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value> + Send + Sync>> {
        // Extract module and function from foreach.do block
        let foreach_def = listen_task
            .foreach
            .as_ref()
            .ok_or_else(|| Error::Configuration { message: "Listen task requires 'foreach' block".to_string() })?;

        // Get the do_ map from the foreach block
        let do_map = foreach_def
            .do_
            .as_ref()
            .ok_or_else(|| Error::Configuration { message: "Listen task 'foreach' requires 'do' block".to_string() })?;

        // Get the first task entry from the map
        let first_entry = do_map
            .entries
            .first()
            .ok_or_else(|| Error::Configuration { message: "Listen task 'foreach.do' requires at least one task".to_string() })?;

        // Get the first (and likely only) task from the entry
        let (_task_name, task_def) = first_entry
            .iter()
            .next()
            .ok_or_else(|| Error::Configuration { message: "Empty task entry in foreach.do".to_string() })?;

        // Extract call task details
        if let TaskDefinition::Call(call_task) = task_def {
            // Get the call type (python, etc.)
            let call_type = &call_task.call;

            // Get module and function from 'with' attributes
            let with_attrs = call_task
                .with
                .as_ref()
                .ok_or_else(|| Error::Configuration { message: "Call task requires 'with' attributes".to_string() })?;

            let module = with_attrs
                .get("module")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Configuration { message: "Call task requires 'module' in 'with' attributes".to_string() })?;

            let function = with_attrs
                .get("function")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Configuration { message: "Call task requires 'function' in 'with' attributes".to_string() })?;

            // Create executor based on call type
            if call_type == "python" {
                let python_executor = Arc::new(PythonExecutor::new());
                let module_owned = module.to_string();
                let function_owned = function.to_string();

                let handler: Arc<
                    dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value> + Send + Sync,
                > = Arc::new(
                    move |payload: serde_json::Value| -> crate::listeners::Result<serde_json::Value> {
                        // Load and call the Python handler
                        // Loading happens at request time, not initialization time,
                        // so PYTHONPATH can be set by the test environment
                        let func = python_executor.load_function(&module_owned, &function_owned)
                            .map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to load Python function: {}", e) })?;
                        let result = python_executor.execute_function(&func, &[payload])
                            .map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to execute Python function: {}", e) })?;
                        Ok(result)
                    },
                );

                Ok(handler)
            } else if call_type == "typescript" {
                let ts_executor = Arc::new(TypeScriptExecutor::new());
                let module_path = module.to_string(); // For TypeScript, this is a file path
                let function_owned = function.to_string();

                let handler: Arc<
                    dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value> + Send + Sync,
                > = Arc::new(
                    move |payload: serde_json::Value| -> crate::listeners::Result<serde_json::Value> {
                        // Run TypeScript execution in a blocking task to avoid runtime-within-runtime panic
                        // rustyscript creates its own Tokio runtime internally
                        let module_path_clone = module_path.clone();
                        let function_clone = function_owned.clone();
                        let executor_clone = ts_executor.clone();

                        let result = tokio::task::block_in_place(|| {
                            executor_clone.execute_function(
                                &module_path_clone,
                                &function_clone,
                                &[payload],
                            )
                        }).map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to execute TypeScript function: {}", e) })?;

                        Ok(result)
                    },
                );

                Ok(handler)
            } else {
                Err(Error::Configuration { message: format!("Unsupported call type: {}. Only 'python' and 'typescript' are currently supported", call_type) })
            }
        } else {
            Err(Error::Configuration { message: "First task in foreach.do must be a Call task".to_string() })
        }
    }

    async fn exec_task(
        &self,
        task_name: &str,
        task: &TaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Determine task type for display
        let task_type = match task {
            TaskDefinition::Call(_) => "Call",
            TaskDefinition::Set(_) => "Set",
            TaskDefinition::Fork(_) => "Fork",
            TaskDefinition::Run(_) => "Run",
            TaskDefinition::Do(_) => "Do",
            TaskDefinition::For(_) => "For",
            TaskDefinition::Switch(_) => "Switch",
            TaskDefinition::Try(_) => "Try",
            TaskDefinition::Emit(_) => "Emit",
            TaskDefinition::Raise(_) => "Raise",
            TaskDefinition::Wait(_) => "Wait",
            TaskDefinition::Listen(_) => "Listen",
        };

        // Format task start
        output::format_task_start(task_name, task_type);

        // Show current context
        let current_context = ctx.data.read().await.clone();
        output::format_task_context(&current_context);

        // Apply input filtering if specified
        let _has_input_filter = self.apply_input_filter(task, ctx).await?;

        // Show input after filtering
        let input_data = ctx.data.read().await.clone();
        output::format_task_input(&input_data);

        // Execute the task
        let result = match task {
            TaskDefinition::Call(call_task) => self.exec_call_task(task_name, call_task, ctx).await,
            TaskDefinition::Set(set_task) => self.exec_set_task(task_name, set_task, ctx).await,
            TaskDefinition::Fork(fork_task) => self.exec_fork_task(task_name, fork_task, ctx).await,
            TaskDefinition::Run(run_task) => self.exec_run_task(task_name, run_task, ctx).await,
            TaskDefinition::Do(do_task) => self.exec_do_task(task_name, do_task, ctx).await,
            TaskDefinition::For(for_task) => self.exec_for_task(task_name, for_task, ctx).await,
            TaskDefinition::Switch(switch_task) => {
                self.exec_switch_task(task_name, switch_task, ctx).await
            }
            TaskDefinition::Raise(raise_task) => {
                self.exec_raise_task(task_name, raise_task, ctx).await
            }
            TaskDefinition::Try(try_task) => self.exec_try_task(task_name, try_task, ctx).await,
            TaskDefinition::Emit(emit_task) => self.exec_emit_task(task_name, emit_task, ctx).await,
            TaskDefinition::Listen(listen_task) => {
                self.exec_listen_task(task_name, listen_task, ctx).await
            }
            _ => {
                println!("  Task type not yet implemented, returning empty result");
                Ok(serde_json::json!({}))
            }
        };

        // Note: We don't restore the original context after input filtering
        // because task outputs (via ctx.merge) should be preserved
        result
    }

    async fn apply_input_filter(&self, task: &TaskDefinition, ctx: &Context) -> Result<bool> {
        // Get the common task fields to check for input.from
        let input_config = match task {
            TaskDefinition::Call(t) => t.common.input.as_ref(),
            TaskDefinition::Set(t) => t.common.input.as_ref(),
            TaskDefinition::Fork(t) => t.common.input.as_ref(),
            TaskDefinition::Run(t) => t.common.input.as_ref(),
            TaskDefinition::Do(t) => t.common.input.as_ref(),
            TaskDefinition::For(t) => t.common.input.as_ref(),
            TaskDefinition::Switch(t) => t.common.input.as_ref(),
            TaskDefinition::Try(t) => t.common.input.as_ref(),
            TaskDefinition::Emit(t) => t.common.input.as_ref(),
            TaskDefinition::Raise(t) => t.common.input.as_ref(),
            _ => None,
        };

        if let Some(input) = input_config {
            if let Some(from_expr) = &input.from {
                if let Some(expr_str) = from_expr.as_str() {
                    let current_data = ctx.data.read().await.clone();
                    let filtered =
                        crate::expressions::evaluate_jq_expression(expr_str, &current_data)?;
                    *ctx.data.write().await = filtered;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Try to load and execute a function from a catalog
    async fn try_load_catalog_function(
        &self,
        function_name: &str,
        with_params: &HashMap<String, serde_json::Value>,
        ctx: &Context,
    ) -> Result<Option<serde_json::Value>> {
        // Parse the function reference to determine if it's a catalog function
        // Formats supported:
        // 1. "function-name:version" - lookup in catalog
        // 2. "https://..." - direct URL to function.yaml
        // 3. "file://..." - direct file path to function.yaml

        let function_url = if function_name.starts_with("http://")
            || function_name.starts_with("https://")
        {
            // Direct HTTP(S) URL
            function_name.to_string()
        } else if function_name.starts_with("file://") {
            // Direct file URL
            function_name.to_string()
        } else if function_name.contains(':') {
            // Catalog reference: "function-name:version"
            let parts: Vec<&str> = function_name.split(':').collect();
            if parts.len() != 2 {
                return Err(Error::Configuration { message: format!("Invalid catalog function reference: {}", function_name) });
            }
            let (name, version) = (parts[0], parts[1]);

            // Look up in catalogs
            let catalogs = match ctx
                .workflow
                .use_
                .as_ref()
                .and_then(|use_| use_.catalogs.as_ref())
            {
                Some(catalogs) => catalogs,
                None => return Ok(None), // No catalogs defined
            };

            // Try to find in any catalog
            let mut function_url = None;
            for (_catalog_name, catalog) in catalogs {
                // Extract URI from the endpoint enum
                use serverless_workflow_core::models::resource::OneOfEndpointDefinitionOrUri;
                let catalog_uri = match &catalog.endpoint {
                    OneOfEndpointDefinitionOrUri::Uri(uri) => uri.as_str(),
                    OneOfEndpointDefinitionOrUri::Endpoint(endpoint_def) => &endpoint_def.uri,
                };

                // Build function URL based on catalog structure
                let url = if catalog_uri.starts_with("file://") {
                    let base_path = catalog_uri.strip_prefix("file://").unwrap();
                    format!("file://{}/{}/{}/function.yaml", base_path, name, version)
                } else if catalog_uri.starts_with("http://") || catalog_uri.starts_with("https://")
                {
                    // For HTTP catalogs, follow the structure: {catalog}/functions/{name}/{version}/function.yaml
                    format!(
                        "{}/functions/{}/{}/function.yaml",
                        catalog_uri.trim_end_matches('/'),
                        name,
                        version
                    )
                } else {
                    return Err(Error::Configuration { message: format!("Unsupported catalog URI scheme: {}", catalog_uri) });
                };

                function_url = Some(url);
                break; // Use first catalog for now
            }

            match function_url {
                Some(url) => url,
                None => return Ok(None), // Not found in catalogs
            }
        } else {
            // Not a catalog function reference
            return Ok(None);
        };

        // Load the function definition
        let function_content = if function_url.starts_with("file://") {
            // Local file
            let path = function_url.strip_prefix("file://").unwrap();
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| Error::Io { source: e })?
        } else {
            // HTTP(S) URL
            let response = reqwest::get(&function_url).await.map_err(|e| {
                Error::TaskExecution { message: format!("Failed to fetch catalog function from {}: {}", function_url, e) }
            })?;

            if !response.status().is_success() {
                return Err(Error::TaskExecution { message: format!("Failed to fetch catalog function from {}: HTTP {}", function_url, response.status()) });
            }

            response.text().await.map_err(|e| {
                Error::TaskExecution { message: format!("Failed to read catalog function response from {}: {}", function_url, e) }
            })?
        };

        // Parse the workflow definition
        let function_workflow: WorkflowDefinition = serde_yaml::from_str(&function_content)
            .map_err(|e| Error::Configuration { message: format!("Failed to parse catalog function {}: {}", function_name, e) })?;

        // Execute the catalog function as a nested workflow with the provided inputs
        let input_data = serde_json::to_value(with_params)?;

        // Run the nested workflow (use Box::pin to avoid infinite-sized future)
        let result = Box::pin(self.run_instance(function_workflow, None, input_data)).await?;

        Ok(Some(result))
    }

    async fn exec_call_task(
        &self,
        task_name: &str,
        call_task: &serverless_workflow_core::models::task::CallTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        let with_params = call_task.with.clone().unwrap_or_default();

        // Evaluate expressions in with parameters
        let current_data = ctx.data.read().await.clone();
        let evaluated_with_params_value = crate::expressions::evaluate_value_with_input(
            &serde_json::to_value(&with_params)?,
            &current_data,
            &ctx.initial_input,
        )?;

        // Convert back to HashMap
        let evaluated_with_params: HashMap<String, serde_json::Value> =
            serde_json::from_value(evaluated_with_params_value.clone())?;

        let params = evaluated_with_params_value.clone();
        let cache_key = compute_cache_key(task_name, &params);

        if let Some(cached) = ctx.cache.get(&cache_key).await? {
            output::format_cache_hit(
                task_name,
                &cache_key,
                Some(&cached.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()),
            );
            return Ok(cached.output);
        }

        output::format_cache_miss(task_name, &cache_key);

        ctx.persistence
            .save_event(WorkflowEvent::TaskStarted {
                instance_id: ctx.instance_id.clone(),
                task_name: task_name.to_string(),
                timestamp: Utc::now(),
            })
            .await?;

        // Resolve the function definition from workflow.use_.functions
        // If not found, check catalogs
        // If still not found, assume it's a built-in protocol (http, grpc, etc.)
        let function_name = &call_task.call;

        // First check user-defined functions
        let function_result = if let Some(function_def) = ctx
            .workflow
            .use_
            .as_ref()
            .and_then(|use_| use_.functions.as_ref())
            .and_then(|funcs| funcs.get(function_name))
        {
            // User-defined function
            let (call_type, func_params) = match function_def {
                TaskDefinition::Call(call_def) => {
                    (&call_def.call, call_def.with.clone().unwrap_or_default())
                }
                _ => return Err(Error::Configuration { message: format!("Function {} is not a call task", function_name) }),
            };
            let mut merged_params = func_params;
            merged_params.extend(evaluated_with_params.clone());

            let executor = self
                .executors
                .get(call_type.as_str())
                .ok_or(Error::TaskExecution { message: format!("No executor for call type: {}", call_type) })?;

            let final_params = serde_json::to_value(&merged_params)?;
            executor.exec(task_name, &final_params, ctx).await?
        } else if let Some(catalog_result) = self
            .try_load_catalog_function(function_name, &evaluated_with_params, ctx)
            .await?
        {
            // Catalog function - execute as nested workflow
            catalog_result
        } else {
            // Built-in protocol
            let executor = self
                .executors
                .get(function_name.as_str())
                .ok_or(Error::TaskExecution { message: format!("No executor for call type: {}", function_name) })?;

            let final_params = serde_json::to_value(&evaluated_with_params)?;
            executor.exec(task_name, &final_params, ctx).await?
        };

        let mut result = function_result;

        // Apply output filtering if specified
        let has_output_filter = if let Some(output_config) = &call_task.common.output {
            if let Some(as_expr) = &output_config.as_ {
                if let Some(expr_str) = as_expr.as_str() {
                    // Evaluate the jq expression on the result
                    result = crate::expressions::evaluate_jq_expression(expr_str, &result)?;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        // If there's an output filter, merge the result directly into context (not nested)
        // This allows the filtered result to be at the root level of the workflow output
        if has_output_filter {
            if let serde_json::Value::Object(map) = &result {
                for (key, value) in map {
                    ctx.merge(key, value.clone()).await;
                }
            } else {
                // If result is a scalar, store it with the task name
                // Track that this task produced a scalar output so we can unwrap it later
                ctx.merge(task_name, result.clone()).await;
                ctx.scalar_output_tasks
                    .write()
                    .await
                    .insert(task_name.to_string());
            }
        }

        let cache_entry = CacheEntry {
            key: cache_key.clone(),
            inputs: params,
            output: result.clone(),
            timestamp: Utc::now(),
        };
        ctx.cache.set(cache_entry).await?;

        Ok(result)
    }

    async fn exec_set_task(
        &self,
        task_name: &str,
        set_task: &serverless_workflow_core::models::task::SetTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Get current context data for expression evaluation
        let current_data = ctx.data.read().await.clone();

        for (key, value) in set_task.set.iter() {
            // Evaluate expressions in the value using current context and initial input
            let evaluated_value = crate::expressions::evaluate_value_with_input(
                value,
                &current_data,
                &ctx.initial_input,
            )?;
            ctx.merge(key, evaluated_value.clone()).await;
        }
        Ok(serde_json::to_value(&set_task.set)?)
    }

    async fn exec_run_task(
        &self,
        task_name: &str,
        run_task: &serverless_workflow_core::models::task::RunTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        let params = serde_json::to_value(&run_task.run)?;
        let cache_key = compute_cache_key(task_name, &params);

        if let Some(cached) = ctx.cache.get(&cache_key).await? {
            output::format_cache_hit(
                task_name,
                &cache_key,
                Some(&cached.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()),
            );
            return Ok(cached.output);
        }

        output::format_cache_miss(task_name, &cache_key);

        ctx.persistence
            .save_event(WorkflowEvent::TaskStarted {
                instance_id: ctx.instance_id.clone(),
                task_name: task_name.to_string(),
                timestamp: Utc::now(),
            })
            .await?;

        // Check what type of run task this is
        let result = if let Some(workflow_def) = run_task.run.workflow.as_ref() {
            // Workflow execution
            let workflow_key = format!(
                "{}/{}/{}",
                workflow_def.namespace, workflow_def.name, workflow_def.version
            );

            // Look up workflow from registry
            let registry = self.workflow_registry.read().await;
            let workflow = registry
                .get(&workflow_key)
                .ok_or_else(|| Error::Configuration { message: format!("Workflow not found in registry: {}", workflow_key) })?
                .clone();
            drop(registry);

            // Get input data for the nested workflow
            let input_data = workflow_def.input.clone().unwrap_or(serde_json::json!({}));

            // Evaluate input data against current context
            let current_data = ctx.data.read().await.clone();
            let evaluated_input = crate::expressions::evaluate_value_with_input(
                &input_data,
                &current_data,
                &ctx.initial_input,
            )?;

            // Execute the nested workflow
            let instance_id = self.start_with_input(workflow, evaluated_input).await?;

            // Wait for completion if await is true (default)
            let should_await = run_task.run.await_.unwrap_or(true);
            if should_await {
                self.wait_for_completion(&instance_id, std::time::Duration::from_secs(300))
                    .await?
            } else {
                serde_json::json!({ "instance_id": instance_id })
            }
        } else if let Some(script) = run_task.run.script.as_ref() {
            // Script execution
            let executor = self
                .executors
                .get("python")
                .ok_or(Error::TaskExecution { message: "No python executor found".to_string() })?;

            // Get script code - either inline or from external source
            let script_code = if let Some(source) = script.source.as_ref() {
                // Load from external source
                use serverless_workflow_core::models::resource::OneOfEndpointDefinitionOrUri;
                let source_uri = match &source.endpoint {
                    OneOfEndpointDefinitionOrUri::Uri(uri) => uri.as_str(),
                    OneOfEndpointDefinitionOrUri::Endpoint(endpoint) => &endpoint.uri,
                };

                if source_uri.starts_with("file://") {
                    // Load from local file
                    let file_path = source_uri.strip_prefix("file://").unwrap();
                    tokio::fs::read_to_string(file_path)
                        .await
                        .map_err(|e| Error::Io { source: e })?
                } else if source_uri.starts_with("http://") || source_uri.starts_with("https://") {
                    // Load from HTTP(S)
                    let response = reqwest::get(source_uri).await.map_err(|e| {
                        Error::TaskExecution { message: format!("Failed to fetch script from {}: {}", source_uri, e) }
                    })?;

                    if !response.status().is_success() {
                        return Err(Error::TaskExecution { message: format!("Failed to fetch script from {}: HTTP {}", source_uri, response.status()) });
                    }

                    response.text().await.map_err(|e| {
                        Error::TaskExecution { message: format!("Failed to read script response from {}: {}", source_uri, e) }
                    })?
                } else {
                    return Err(Error::Configuration { message: format!("Unsupported source URI scheme: {}", source_uri) });
                }
            } else if let Some(inline_code) = script.code.as_ref() {
                // Use inline code
                inline_code.clone()
            } else {
                return Err(Error::Configuration { message: "Script must have either 'code' or 'source' defined".to_string() });
            };

            // Get script arguments if provided and evaluate them against context
            let current_data = ctx.data.read().await.clone();
            let arguments = if let Some(args) = script.arguments.as_ref() {
                crate::expressions::evaluate_value_with_input(
                    &serde_json::to_value(args)?,
                    &current_data,
                    &ctx.initial_input,
                )?
            } else {
                // No arguments specified
                serde_json::json!({})
            };

            // Execute script with arguments injected as globals
            let script_params = serde_json::json!({
                "script": script_code,
                "arguments": arguments
            });

            executor.exec(task_name, &script_params, ctx).await?
        } else {
            // Other run types (container, shell, etc.) not yet implemented
            serde_json::json!({})
        };

        let cache_entry = CacheEntry {
            key: cache_key.clone(),
            inputs: params,
            output: result.clone(),
            timestamp: Utc::now(),
        };
        ctx.cache.set(cache_entry).await?;

        Ok(result)
    }

    async fn exec_fork_task(
        &self,
        task_name: &str,
        fork_task: &serverless_workflow_core::models::task::ForkTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        let mut results = HashMap::new();

        // Check if compete mode is enabled - use different future types
        if fork_task.fork.compete {
            // In compete mode, use boxed futures for select_all (requires Unpin)
            let mut branch_futures = Vec::new();

            for entry in &fork_task.fork.branches.entries {
                for (branch_name, branch_task) in entry {
                    let branch_name = branch_name.clone();
                    let branch_task = branch_task.clone();
                    let ctx = ctx.clone();
                    let engine = self as *const Self;

                    let future = Box::pin(async move {
                        let engine_ref = unsafe { &*engine };
                        let result = engine_ref
                            .exec_task(&branch_name, &branch_task, &ctx)
                            .await?;
                        Ok::<_, Error>((branch_name, result))
                    });
                    branch_futures.push(future);
                }
            }

            if !branch_futures.is_empty() {
                let (result, _index, _remaining) =
                    futures::future::select_all(branch_futures).await;
                let (branch_name, branch_result) = result?;
                results.insert(branch_name, branch_result);
            }
        } else {
            // In normal mode, plain futures work fine with join_all
            let mut branch_futures = Vec::new();

            for entry in &fork_task.fork.branches.entries {
                for (branch_name, branch_task) in entry {
                    let branch_name = branch_name.clone();
                    let branch_task = branch_task.clone();
                    let ctx = ctx.clone();
                    let engine = self as *const Self;

                    let future = async move {
                        let engine_ref = unsafe { &*engine };
                        let result = engine_ref
                            .exec_task(&branch_name, &branch_task, &ctx)
                            .await?;
                        Ok::<_, Error>((branch_name, result))
                    };
                    branch_futures.push(future);
                }
            }

            let branch_results = futures::future::join_all(branch_futures).await;

            for result in branch_results {
                let (branch_name, branch_result) = result?;
                results.insert(branch_name, branch_result);
            }
        }

        Ok(serde_json::to_value(&results)?)
    }

    async fn exec_do_task(
        &self,
        task_name: &str,
        do_task: &serverless_workflow_core::models::task::DoTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        let mut results = serde_json::Map::new();

        // Execute subtasks sequentially in order
        for entry in &do_task.do_.entries {
            for (subtask_name, subtask) in entry {
                // Box the recursive call to avoid infinite sized future
                let result = Box::pin(self.exec_task(subtask_name, subtask, ctx)).await?;
                results.insert(subtask_name.clone(), result);
            }
        }

        Ok(serde_json::Value::Object(results))
    }

    async fn exec_for_task(
        &self,
        task_name: &str,
        for_task: &serverless_workflow_core::models::task::ForTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Get current context data
        let current_data = ctx.data.read().await.clone();

        // Evaluate the 'in' expression to get the collection to iterate over
        let collection_expr = &for_task.for_.in_;
        let collection = crate::expressions::evaluate_jq(collection_expr, &current_data)?;

        // Get the collection as an array
        let items = collection.as_array().ok_or(Error::TaskExecution { message: format!("For loop 'in' expression must evaluate to an array, got: {:?}", collection) })?;

        // Get the iteration variable name (e.g., "color")
        let item_var = &for_task.for_.each;

        // Get the index variable name (defaults to "index" if not specified)
        let index_var = for_task.for_.at.as_deref().unwrap_or("index");

        // Iterate over the collection
        for (index, item) in items.iter().enumerate() {
            // Get current accumulated state (includes updates from previous iterations)
            let accumulated_data = ctx.data.read().await.clone();

            // Inject iteration variables into the current state
            let mut iteration_data = accumulated_data;
            if let Some(obj) = iteration_data.as_object_mut() {
                // Store the item and index as variables (without $ prefix, jq will handle $ reference)
                obj.insert(item_var.clone(), item.clone());
                obj.insert(index_var.to_string(), serde_json::json!(index));
            }

            // Update context with iteration variables
            {
                let mut data_guard = ctx.data.write().await;
                *data_guard = iteration_data;
            }

            // Execute the do tasks for this iteration
            for entry in &for_task.do_.entries {
                for (subtask_name, subtask) in entry {
                    Box::pin(self.exec_task(subtask_name, subtask, ctx)).await?;
                }
            }

            // Remove iteration variables but keep accumulated changes
            {
                let mut data_guard = ctx.data.write().await;
                if let Some(obj) = data_guard.as_object_mut() {
                    obj.remove(item_var);
                    obj.remove(index_var);
                }
            }
        }

        Ok(serde_json::json!({}))
    }

    async fn exec_switch_task(
        &self,
        task_name: &str,
        switch_task: &serverless_workflow_core::models::task::SwitchTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Get current context data
        let current_data = ctx.data.read().await.clone();

        // Evaluate each case in order
        for entry in &switch_task.switch.entries {
            for (case_name, case_def) in entry {
                // If there's a 'when' condition, evaluate it
                let matches = if let Some(when_expr) = &case_def.when {
                    // Evaluate the condition expression
                    let result = crate::expressions::evaluate_jq(when_expr, &current_data)?;

                    // Check if the result is truthy
                    let matches = match result {
                        serde_json::Value::Bool(b) => b,
                        serde_json::Value::Null => false,
                        _ => true, // Non-null, non-bool values are truthy
                    };

                    matches
                } else {
                    // No 'when' condition means this is a default case
                    true
                };

                if matches {
                    // Set the next task to the matched case's 'then' target
                    if let Some(then_target) = &case_def.then {
                        *ctx.next_task.write().await = Some(then_target.clone());
                    }
                    // The switch task doesn't execute anything itself
                    // It just evaluates conditions and the graph will follow the 'then' transition
                    // Return empty result since the actual work is done by the transitioned-to task
                    return Ok(serde_json::json!({}));
                }
            }
        }

        // No cases matched - check if there's a common 'then' transition
        if let Some(then_target) = &switch_task.common.then {
            *ctx.next_task.write().await = Some(then_target.clone());
        } else {
        }
        Ok(serde_json::json!({}))
    }

    async fn exec_raise_task(
        &self,
        task_name: &str,
        raise_task: &serverless_workflow_core::models::task::RaiseTaskDefinition,
        _ctx: &Context,
    ) -> Result<serde_json::Value> {
        use serverless_workflow_core::models::error::OneOfErrorDefinitionOrReference;

        // Extract the error definition
        let error_def = match &raise_task.raise.error {
            OneOfErrorDefinitionOrReference::Error(err) => err,
            OneOfErrorDefinitionOrReference::Reference(ref_name) => {
                return Err(Error::Configuration { message: format!("Error references not yet implemented: {}", ref_name) });
            }
        };

        // Build the error object according to the spec
        let mut error_obj = serde_json::json!({
            "type": error_def.type_,
            "title": error_def.title,
            "status": error_def.status,
        });

        // Add optional fields if present
        if let Some(detail) = &error_def.detail {
            error_obj.as_object_mut().unwrap().insert(
                "detail".to_string(),
                serde_json::Value::String(detail.clone()),
            );
        }

        // Add the instance field - this should be the path to the task in the workflow
        // The path format is /do/index/taskName
        let task_path = format!("/do/0/{}", task_name);
        error_obj
            .as_object_mut()
            .unwrap()
            .insert("instance".to_string(), serde_json::Value::String(task_path));

        // Serialize the error to a JSON string for the error message
        let error_json = serde_json::to_string(&error_obj)?;

        // Return an error with the JSON-serialized error object
        Err(Error::TaskExecution { message: error_json })
    }

    async fn exec_try_task(
        &self,
        task_name: &str,
        try_task: &serverless_workflow_core::models::task::TryTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Execute the tasks in the try block
        let mut try_result = Ok(serde_json::json!({}));

        for entry in &try_task.try_.entries {
            for (subtask_name, subtask) in entry {
                println!("    Executing try subtask: {}", subtask_name);

                // Box the async call to avoid infinite recursion
                let exec_future = self.exec_task(subtask_name, subtask, ctx);
                match Box::pin(exec_future).await {
                    Ok(result) => {
                        // Merge the result into context
                        ctx.merge(subtask_name, result.clone()).await;
                        try_result = Ok(result);
                    }
                    Err(e) => {
                        // An error occurred - check if it should be caught

                        // Try to parse the error as JSON to check if it matches the filter
                        let error_obj: serde_json::Value = if let Ok(parsed) =
                            serde_json::from_str(&e.to_string())
                        {
                            parsed
                        } else {
                            // If not JSON, create a generic error object
                            serde_json::json!({
                                "type": "https://serverlessworkflow.io/dsl/errors/types/runtime",
                                "status": 500,
                                "title": "Runtime Error",
                                "detail": e.to_string(),
                                "instance": format!("/do/0/{}/try/0/{}", task_name, subtask_name)
                            })
                        };

                        // Check if the error matches the catch filter
                        let should_catch = self.should_catch_error(&error_obj, &try_task.catch);

                        if should_catch {
                            // Store the error in context using the specified variable name
                            let error_var_name = try_task.catch.as_.as_deref().unwrap_or("error");
                            ctx.merge(error_var_name, error_obj.clone()).await;

                            // Execute the catch handler tasks if defined
                            if let Some(ref catch_tasks) = try_task.catch.do_ {
                                for catch_entry in &catch_tasks.entries {
                                    for (catch_task_name, catch_task) in catch_entry {
                                        // Box the async call to avoid infinite recursion
                                        let exec_future =
                                            self.exec_task(catch_task_name, catch_task, ctx);
                                        let catch_result = Box::pin(exec_future).await?;
                                        ctx.merge(catch_task_name, catch_result).await;
                                    }
                                }
                            }

                            // Try task completes successfully after catching and handling the error
                            return Ok(serde_json::json!({}));
                        } else {
                            // Error doesn't match the filter, propagate it
                            return Err(e);
                        }
                    }
                }
            }
        }

        try_result
    }

    fn should_catch_error(
        &self,
        error: &serde_json::Value,
        catch_def: &serverless_workflow_core::models::task::ErrorCatcherDefinition,
    ) -> bool {
        // If no error filter is defined, catch all errors
        let Some(ref error_filter) = catch_def.errors else {
            return true;
        };

        // If no 'with' filter is defined, catch all errors
        let Some(ref with_filter) = error_filter.with else {
            return true;
        };

        // Check if all filter properties match
        for (key, expected_value) in with_filter {
            let actual_value = error.get(key);

            match actual_value {
                Some(actual) => {
                    // Compare values - need to handle different types
                    if !values_match(expected_value, actual) {
                        return false;
                    }
                }
                None => {
                    return false;
                }
            }
        }

        true
    }

    async fn exec_emit_task(
        &self,
        _task_name: &str,
        emit_task: &serverless_workflow_core::models::task::EmitTaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Get current context data for expression evaluation
        let current_data = ctx.data.read().await.clone();

        // Evaluate the event attributes
        let mut event_data = serde_json::Map::new();

        // CloudEvents standard fields
        // Generate a unique ID for the event
        event_data.insert(
            "id".to_string(),
            serde_json::json!(uuid::Uuid::new_v4().to_string()),
        );

        // CloudEvents spec version
        event_data.insert("specversion".to_string(), serde_json::json!("1.0"));

        // Add timestamp
        event_data.insert(
            "time".to_string(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );

        // Process the 'with' attributes from the event definition
        for (key, value) in &emit_task.emit.event.with {
            let evaluated_value = crate::expressions::evaluate_value_with_input(
                value,
                &current_data,
                &ctx.initial_input,
            )?;
            event_data.insert(key.clone(), evaluated_value);
        }

        let result = serde_json::Value::Object(event_data);

        // Merge each field of the event into the context (not nested under task name)
        if let serde_json::Value::Object(map) = &result {
            for (key, value) in map {
                ctx.merge(key, value.clone()).await;
            }
        }

        Ok(result)
    }

    async fn exec_listen_task(
        &self,
        _task_name: &str,
        _listen_task: &serverless_workflow_core::models::task::ListenTaskDefinition,
        _ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Listen tasks are now initialized at workflow startup via initialize_listeners()
        // This method is kept for compatibility but does nothing during execution
        // The listener is already running and will continue to run until workflow completes
        Ok(serde_json::json!({"status": "already_listening"}))
    }
}

/// Convert a prost-reflect DynamicMessage to JSON
fn dynamic_message_to_json(msg: &prost_reflect::DynamicMessage) -> Result<serde_json::Value> {
    use prost_reflect::{ReflectMessage, Value};

    let mut json_map = serde_json::Map::new();
    let descriptor = msg.descriptor();

    for field in descriptor.fields() {
        let value = msg.get_field(&field);
        let json_value = match value.as_ref() {
            Value::I32(i) => serde_json::json!(i),
            Value::I64(i) => serde_json::json!(i),
            Value::U32(u) => serde_json::json!(u),
            Value::U64(u) => serde_json::json!(u),
            Value::F32(f) => serde_json::json!(f),
            Value::F64(f) => serde_json::json!(f),
            Value::Bool(b) => serde_json::json!(b),
            Value::String(s) => serde_json::json!(s),
            Value::Bytes(b) => serde_json::json!(base64::encode(b)),
            _ => serde_json::Value::Null,
        };
        json_map.insert(field.name().to_string(), json_value);
    }

    Ok(serde_json::Value::Object(json_map))
}

/// Convert JSON to a prost-reflect DynamicMessage using a message descriptor
fn json_to_dynamic_message(
    json: &serde_json::Value,
    descriptor: prost_reflect::MessageDescriptor,
) -> Result<prost_reflect::DynamicMessage> {
    use prost_reflect::{DynamicMessage, ReflectMessage, Value};

    let mut msg = DynamicMessage::new(descriptor.clone());

    if let serde_json::Value::Object(map) = json {
        for (key, value) in map {
            if let Some(field) = descriptor.get_field_by_name(key) {
                let field_value = match value {
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value::I32(i as i32)
                        } else if let Some(f) = n.as_f64() {
                            Value::F64(f)
                        } else {
                            continue;
                        }
                    }
                    serde_json::Value::String(s) => Value::String(s.clone()),
                    serde_json::Value::Bool(b) => Value::Bool(*b),
                    _ => continue,
                };
                msg.set_field(&field, field_value);
            }
        }
    }

    Ok(msg)
}

fn values_match(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    // Handle different value types
    match (expected, actual) {
        (serde_json::Value::Number(e), serde_json::Value::Number(a)) => {
            // Handle number comparisons where one might be an integer and the other a float
            e.as_f64() == a.as_f64()
        }
        (serde_json::Value::String(e), serde_json::Value::String(a)) => e == a,
        (serde_json::Value::Bool(e), serde_json::Value::Bool(a)) => e == a,
        (serde_json::Value::Null, serde_json::Value::Null) => true,
        _ => expected == actual,
    }
}

fn get_task_transitions(task: &TaskDefinition) -> Vec<String> {
    match task {
        TaskDefinition::Call(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Set(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Fork(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Switch(t) => {
            let mut transitions = Vec::new();
            for entry in &t.switch.entries {
                for (_, case) in entry {
                    if let Some(then) = &case.then {
                        transitions.push(then.clone());
                    }
                }
            }
            transitions
        }
        TaskDefinition::Do(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        _ => vec![],
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
            _ => return Err(Error::Configuration { message: format!("Unknown visualization tool: {}", tool) }),
        };

        // Check availability
        if !provider.is_available()? {
            return Err(Error::Configuration { message: format!("{} is not installed or not available", provider.name()) });
        }

        // Render diagram with execution state
        provider.render(workflow, output_path, format, Some(&execution_state))?;

        Ok(())
    }
}
