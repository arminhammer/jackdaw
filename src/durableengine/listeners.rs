use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serverless_workflow_core::models::task::{ListenTaskDefinition, TaskDefinition};
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::collections::HashMap;
use std::sync::Arc;

use crate::listeners::{EventSource, Listener, grpc::GrpcListener, http::HttpListener};
use crate::providers::executors::{PythonExecutor, TypeScriptExecutor};

use super::{DurableEngine, Error, Result};

impl DurableEngine {
    /// Initialize all listeners from the workflow before task execution begins
    ///
    /// This scans the workflow for all Listen tasks, groups them by bind address,
    /// and starts all listeners together with their complete route tables
    pub(super) async fn initialize_listeners(&self, workflow: &WorkflowDefinition) -> Result<()> {
        // Collect all HTTP routes grouped by (bind_addr, openapi_path)
        // Key: (bind_addr, openapi_path), Value: Vec of (path, task_name, handler)
        let mut http_routes: HashMap<
            (String, String),
            Vec<(
                String,
                String,
                Arc<
                    dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value>
                        + Send
                        + Sync,
                >,
            )>,
        > = HashMap::new();

        // Collect all gRPC methods grouped by (bind_addr, proto_path, service_name)
        // Key: (bind_addr, proto_path, service_name), Value: Vec of (method_name, task_name, handler)
        let mut grpc_methods: HashMap<
            (String, String, String),
            Vec<(
                String,
                String,
                Arc<
                    dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value>
                        + Send
                        + Sync,
                >,
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
                            .ok_or_else(|| Error::Listener {
                                message: "Invalid HTTP URI".to_string(),
                            })?;

                        let parts: Vec<&str> = without_scheme.splitn(2, '/').collect();
                        let mut bind_addr = parts
                            .first()
                            .ok_or_else(|| Error::Listener {
                                message: "Invalid gRPC URI: missing bind address".to_string(),
                            })?
                            .to_string();

                        // Convert localhost to 127.0.0.1 for SocketAddr parsing
                        if bind_addr.starts_with("localhost:") {
                            bind_addr = bind_addr.replace("localhost:", "127.0.0.1:");
                        }

                        let path = if let Some(path_part) = parts.get(1) {
                            format!("/{path_part}")
                        } else {
                            "/".to_string()
                        };

                        let openapi_path = schema_path_opt.ok_or_else(|| Error::Listener {
                            message: "HTTP listener requires OpenAPI schema".to_string(),
                        })?;

                        // Create handler for this route
                        let handler = self.create_handler_from_listen_task(listen_task)?;

                        // Group by (bind_addr, openapi_path) - different specs can coexist on same port
                        http_routes
                            .entry((bind_addr.clone(), openapi_path.clone()))
                            .or_default()
                            .push((path, task_name.clone(), handler));
                    }
                    // Handle gRPC listeners
                    else if event_source.uri.starts_with("grpc://") {
                        // Parse bind address and method from URI (e.g., grpc://localhost:50051/calculator.Calculator/Add)
                        let uri = &event_source.uri;
                        let without_scheme =
                            uri.strip_prefix("grpc://").ok_or_else(|| Error::Listener {
                                message: "Invalid gRPC URI".to_string(),
                            })?;

                        let parts: Vec<&str> = without_scheme.splitn(2, '/').collect();
                        let mut bind_addr = parts[0].to_string();

                        // Convert localhost to 127.0.0.1 for SocketAddr parsing
                        if bind_addr.starts_with("localhost:") {
                            bind_addr = bind_addr.replace("localhost:", "127.0.0.1:");
                        }

                        // Extract service and method from the path (e.g., "calculator.Calculator/Add")
                        let method_path = parts.get(1).ok_or_else(|| Error::Listener {
                            message: "gRPC URI must include service/method path".to_string(),
                        })?;

                        let method_parts: Vec<&str> = method_path.split('/').collect();
                        if method_parts.len() != 2 {
                            return Err(Error::Listener {
                                message: "gRPC method path must be in format 'service.Name/Method'"
                                    .to_string(),
                            });
                        }
                        let service_name =
                            (*method_parts.first().ok_or_else(|| Error::Listener {
                                message: "Missing service name in gRPC method path".to_string(),
                            })?)
                            .to_string();
                        let method_name =
                            (*method_parts.get(1).ok_or_else(|| Error::Listener {
                                message: "Missing method name in gRPC method path".to_string(),
                            })?)
                            .to_string();

                        let proto_path = schema_path_opt.ok_or_else(|| Error::Listener {
                            message: "gRPC listener requires proto schema".to_string(),
                        })?;

                        // Create handler for this method
                        let handler = self.create_handler_from_listen_task(listen_task)?;

                        // Group by (bind_addr, proto_path, service_name)
                        grpc_methods
                            .entry((bind_addr.clone(), proto_path.clone(), service_name.clone()))
                            .or_default()
                            .push((method_name, task_name.clone(), handler));
                    }
                }
            }
        }

        // Now create all HTTP listeners with their complete route tables
        let mut http_listeners = self.http_listeners.write().await;

        for ((bind_addr, _openapi_path), routes) in http_routes {
            // Build route handlers map
            let mut route_handlers = std::collections::HashMap::new();
            for (path, task_name, handler) in routes {
                route_handlers.insert(path.clone(), handler);
                println!("  Registering route {path} for task {task_name} on {bind_addr}");
            }

            // Create and start the listener with all routes
            let listener = HttpListener::new_multi_route(bind_addr.clone(), route_handlers)?;
            let listener_arc = Arc::new(listener);
            listener_arc.start().await?;

            // Wait a bit for the server to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            http_listeners.insert(bind_addr.clone(), listener_arc);
            println!("  HTTP listener started on {bind_addr}");
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
            let service_descriptor =
                pool.get_service_by_name(&service_name)
                    .ok_or_else(|| Error::Listener {
                        message: format!("Service {service_name} not found in proto file"),
                    })?;

            // Build method handlers map - convert JSON handlers to DynamicMessage handlers
            let mut method_handlers: std::collections::HashMap<
                String,
                Arc<
                    dyn Fn(DynamicMessage) -> crate::listeners::Result<DynamicMessage>
                        + Send
                        + Sync,
                >,
            > = std::collections::HashMap::new();

            for (method_name, task_name, json_handler) in methods {
                println!(
                    "  Registering gRPC method {service_name}/{method_name} for task {task_name} on {bind_addr}"
                );

                // Get method descriptor for this method
                let method_descriptor = service_descriptor
                    .methods()
                    .find(|m| m.name() == method_name)
                    .ok_or_else(|| Error::Listener {
                        message: format!(
                            "Method {method_name} not found in service {service_name}"
                        ),
                    })?;

                let output_descriptor = method_descriptor.output();

                // Clone the Arc so each closure gets its own reference
                let json_handler_clone = json_handler.clone();

                // Create a wrapper that converts DynamicMessage to JSON, calls the JSON handler, then converts back
                let wrapped_handler: Arc<
                    dyn Fn(DynamicMessage) -> crate::listeners::Result<DynamicMessage>
                        + Send
                        + Sync,
                > = Arc::new(
                    move |request_msg: DynamicMessage| -> crate::listeners::Result<DynamicMessage> {
                        // Convert DynamicMessage to JSON
                        let request_json = dynamic_message_to_json(&request_msg);

                        // Call the JSON handler
                        let response_json = json_handler_clone(request_json)?;

                        // Convert JSON response back to DynamicMessage using the output descriptor
                        let response_msg =
                            json_to_dynamic_message(&response_json, &output_descriptor);
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
            println!("  gRPC listener started on {bind_addr}");
        }

        Ok(())
    }

    /// Extract event source and ``OpenAPI`` path from a Listen task
    #[allow(clippy::unused_self)]
    fn extract_listen_source(
        &self,
        listen_task: &ListenTaskDefinition,
    ) -> Result<(serde_json::Value, Option<String>)> {
        let (_event_filter, _with_attrs, source_value) = if let Some(one_filter) =
            &listen_task.listen.to.one
        {
            let with_attrs = one_filter
                .with
                .as_ref()
                .ok_or_else(|| Error::Configuration {
                    message: "Listen task requires 'with' attributes".to_string(),
                })?;
            let source_value = with_attrs
                .get("source")
                .ok_or_else(|| Error::Configuration {
                    message: "Listen task requires 'source' in 'with' attributes".to_string(),
                })?;
            (one_filter, with_attrs, source_value)
        } else if let Some(any_filters) = &listen_task.listen.to.any {
            let first_filter = any_filters.first().ok_or_else(|| Error::Configuration {
                message: "Listen task 'any' requires at least one event filter".to_string(),
            })?;
            let with_attrs = first_filter
                .with
                .as_ref()
                .ok_or_else(|| Error::Configuration {
                    message: "Listen task requires 'with' attributes".to_string(),
                })?;
            let source_value = with_attrs
                .get("source")
                .ok_or_else(|| Error::Configuration {
                    message: "Listen task requires 'source' in 'with' attributes".to_string(),
                })?;
            (first_filter, with_attrs, source_value)
        } else {
            return Err(Error::Configuration {
                message: "Listen task requires either 'one' or 'any' event filter".to_string(),
            });
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
    ///
    /// Extracts the handler module and function from the first Call task in the foreach block
    #[allow(clippy::unused_self)]
    fn create_handler_from_listen_task(
        &self,
        listen_task: &ListenTaskDefinition,
    ) -> Result<
        Arc<dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value> + Send + Sync>,
    > {
        // Extract module and function from foreach.do block
        let foreach_def = listen_task
            .foreach
            .as_ref()
            .ok_or_else(|| Error::Configuration {
                message: "Listen task requires 'foreach' block".to_string(),
            })?;

        // Get the do_ map from the foreach block
        let do_map = foreach_def
            .do_
            .as_ref()
            .ok_or_else(|| Error::Configuration {
                message: "Listen task 'foreach' requires 'do' block".to_string(),
            })?;

        // Get the first task entry from the map
        let first_entry = do_map.entries.first().ok_or_else(|| Error::Configuration {
            message: "Listen task 'foreach.do' requires at least one task".to_string(),
        })?;

        // Get the first (and likely only) task from the entry
        let (_task_name, task_def) =
            first_entry
                .iter()
                .next()
                .ok_or_else(|| Error::Configuration {
                    message: "Empty task entry in foreach.do".to_string(),
                })?;

        // Extract call task details
        if let TaskDefinition::Call(call_task) = task_def {
            // Get the call type (python, etc.)
            let call_type = &call_task.call;

            // Get module and function from 'with' attributes
            let with_attrs = call_task
                .with
                .as_ref()
                .ok_or_else(|| Error::Configuration {
                    message: "Call task requires 'with' attributes".to_string(),
                })?;

            let module = with_attrs
                .get("module")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Configuration {
                    message: "Call task requires 'module' in 'with' attributes".to_string(),
                })?;

            let function = with_attrs
                .get("function")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Configuration {
                    message: "Call task requires 'function' in 'with' attributes".to_string(),
                })?;

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
                            .map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to load Python function: {e}") })?;
                        let result = python_executor.execute_function(&func, &[payload])
                            .map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to execute Python function: {e}") })?;
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
                        }).map_err(|e| crate::listeners::Error::Execution { message: format!("Failed to execute TypeScript function: {e}") })?;

                        Ok(result)
                    },
                );

                Ok(handler)
            } else {
                Err(Error::Configuration {
                    message: format!(
                        "Unsupported call type: {call_type}. Only 'python' and 'typescript' are currently supported"
                    ),
                })
            }
        } else {
            Err(Error::Configuration {
                message: "First task in foreach.do must be a Call task".to_string(),
            })
        }
    }
}

/// Convert a prost-reflect ``DynamicMessage`` to JSON
fn dynamic_message_to_json(msg: &prost_reflect::DynamicMessage) -> serde_json::Value {
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
            Value::Bool(b) => serde_json::json!(b),
            Value::String(s) => serde_json::json!(s),
            Value::Bytes(b) => serde_json::json!(BASE64_STANDARD.encode(b)),
            Value::F32(_)
            | Value::F64(_)
            | Value::EnumNumber(_)
            | Value::Message(_)
            | Value::List(_)
            | Value::Map(_) => serde_json::Value::Null,
        };
        json_map.insert(field.name().to_string(), json_value);
    }

    serde_json::Value::Object(json_map)
}

/// Convert JSON to a prost-reflect ``DynamicMessage`` using a message descriptor
#[allow(clippy::cast_possible_truncation)]
fn json_to_dynamic_message(
    json: &serde_json::Value,
    descriptor: &prost_reflect::MessageDescriptor,
) -> prost_reflect::DynamicMessage {
    use prost_reflect::{DynamicMessage, Value};

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
                    serde_json::Value::Null
                    | serde_json::Value::Array(_)
                    | serde_json::Value::Object(_) => continue,
                };
                msg.set_field(&field, field_value);
            }
        }
    }

    msg
}
