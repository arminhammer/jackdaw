use chrono::Utc;
use snafu::prelude::*;
use std::process::Stdio;

use crate::cache::{CacheEntry, compute_cache_key};
use crate::container::{ContainerConfig, ContainerProvider};
use crate::context::Context;
use crate::output;
use crate::providers::container::DockerProvider;
use crate::task_output::TaskOutputStreamer;

use super::super::{DurableEngine, Error, IoSnafu, Result};

/// Execute a Run task - runs workflows, scripts, containers, or shell commands
pub async fn exec_run_task(
    engine: &DurableEngine,
    task_name: &str,
    run_task: &serverless_workflow_core::models::task::RunTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Evaluate expressions in the run task definition before computing cache key
    // This ensures that expressions like $workflow.id are evaluated to their actual values
    let current_data = ctx.state.data.read().await.clone();
    let params = serde_json::to_value(&run_task.run)?;
    let evaluated_params = crate::expressions::evaluate_value_with_input(
        &params,
        &current_data,
        &ctx.metadata.initial_input,
    )?;

    // Combine task definition with current context data for cache key
    // This ensures that input.from filters affect caching
    let cache_params = serde_json::json!({
        "task": evaluated_params,
        "input": current_data
    });

    let cache_key = compute_cache_key(task_name, &cache_params);

    if let Some(cached) = ctx.services.cache.get(&cache_key).await? {
        output::format_cache_hit(
            task_name,
            &cache_key,
            Some(&cached.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()),
        );
        return Ok(cached.output);
    }

    output::format_cache_miss(task_name, &cache_key);

    // Note: TaskStarted event is now emitted centrally in exec_task()

    // Check what type of run task this is
    let result = if let Some(workflow_def) = run_task.run.workflow.as_ref() {
        // Workflow execution
        let workflow_key = format!(
            "{}/{}/{}",
            workflow_def.namespace, workflow_def.name, workflow_def.version
        );

        // Look up workflow from registry
        let registry = engine.workflow_registry.read().await;
        let workflow = registry
            .get(&workflow_key)
            .ok_or_else(|| Error::Configuration {
                message: format!("Workflow not found in registry: {workflow_key}"),
            })?
            .clone();
        drop(registry);

        // Get input data for the nested workflow
        let input_data = workflow_def.input.clone().unwrap_or(serde_json::json!({}));

        // Evaluate input data against current context
        let current_data = ctx.state.data.read().await.clone();
        let evaluated_input = crate::expressions::evaluate_value_with_input(
            &input_data,
            &current_data,
            &ctx.metadata.initial_input,
        )?;

        // Execute the nested workflow
        let (instance_id, final_data) = engine.start_with_input(workflow, evaluated_input).await?;

        // Wait for completion if await is true (default)
        let should_await = run_task.run.await_.unwrap_or(true);
        if should_await {
            final_data
        } else {
            serde_json::json!({ "instance_id": instance_id })
        }
    } else if let Some(script) = run_task.run.script.as_ref() {
        // Script execution - select executor based on language
        let language = script.language.to_lowercase();

        // Display script parameters instead of generic input
        let current_data = ctx.state.data.read().await.clone();
        let stdin_display = script.stdin.as_ref().and_then(|s| {
            crate::expressions::evaluate_value_with_input(
                &serde_json::Value::String(s.clone()),
                &current_data,
                &ctx.metadata.initial_input,
            )
            .ok()
            .and_then(|v| v.as_str().map(String::from))
        });

        let arguments_display = script.arguments.as_ref().and_then(|args| {
            crate::expressions::evaluate_value_with_input(
                &serde_json::to_value(args).ok()?,
                &current_data,
                &ctx.metadata.initial_input,
            )
            .ok()
        });

        let environment_display = script.environment.as_ref().and_then(|env| {
            let mut evaluated_env = serde_json::Map::new();
            for (key, value) in env {
                if let Some(evaluated) = crate::expressions::evaluate_value_with_input(
                    &serde_json::Value::String(value.clone()),
                    &current_data,
                    &ctx.metadata.initial_input,
                )
                .ok()
                .and_then(|v| v.as_str().map(|s| (key.clone(), s.to_string())))
                {
                    evaluated_env.insert(evaluated.0, serde_json::Value::String(evaluated.1));
                }
            }
            if evaluated_env.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(evaluated_env))
            }
        });

        output::format_run_task_params(
            Some(&language),
            stdin_display.as_deref(),
            arguments_display.as_ref(),
            environment_display.as_ref(),
        );

        let executor = engine
            .executors
            .get(&language)
            .ok_or(Error::TaskExecution {
                message: format!("No executor found for language: {language}"),
            })?;

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
                let file_path =
                    source_uri
                        .strip_prefix("file://")
                        .ok_or_else(|| Error::Configuration {
                            message: format!("Invalid file URI format: {source_uri}"),
                        })?;
                tokio::fs::read_to_string(file_path)
                    .await
                    .context(IoSnafu)?
            } else if source_uri.starts_with("http://") || source_uri.starts_with("https://") {
                // Load from HTTP(S)
                let response =
                    reqwest::get(source_uri)
                        .await
                        .map_err(|e| Error::TaskExecution {
                            message: format!("Failed to fetch script from {source_uri}: {e}"),
                        })?;

                if !response.status().is_success() {
                    return Err(Error::TaskExecution {
                        message: format!(
                            "Failed to fetch script from {}: HTTP {}",
                            source_uri,
                            response.status()
                        ),
                    });
                }

                response.text().await.map_err(|e| Error::TaskExecution {
                    message: format!("Failed to read script response from {source_uri}: {e}"),
                })?
            } else {
                return Err(Error::Configuration {
                    message: format!("Unsupported source URI scheme: {source_uri}"),
                });
            }
        } else if let Some(inline_code) = script.code.as_ref() {
            // Use inline code
            inline_code.clone()
        } else {
            return Err(Error::Configuration {
                message: "Script must have either 'code' or 'source' defined".to_string(),
            });
        };

        // Get script arguments if provided and evaluate them against context
        let current_data = ctx.state.data.read().await.clone();
        let arguments = if let Some(args) = script.arguments.as_ref() {
            crate::expressions::evaluate_value_with_input(
                &serde_json::to_value(args)?,
                &current_data,
                &ctx.metadata.initial_input,
            )?
        } else {
            // No arguments specified
            serde_json::json!([])
        };

        // Get stdin if provided and evaluate it
        let stdin = if let Some(stdin_str) = script.stdin.as_ref() {
            let evaluated = crate::expressions::evaluate_value_with_input(
                &serde_json::Value::String(stdin_str.clone()),
                &current_data,
                &ctx.metadata.initial_input,
            )?;
            evaluated.as_str().map(String::from)
        } else {
            None
        };

        // Get environment variables if provided and evaluate them
        let environment = if let Some(env) = script.environment.as_ref() {
            let mut evaluated_env = serde_json::Map::new();
            for (key, value) in env {
                let evaluated = crate::expressions::evaluate_value_with_input(
                    &serde_json::Value::String(value.clone()),
                    &current_data,
                    &ctx.metadata.initial_input,
                )?;
                if let Some(s) = evaluated.as_str() {
                    evaluated_env.insert(key.clone(), serde_json::Value::String(s.to_string()));
                }
            }
            Some(serde_json::Value::Object(evaluated_env))
        } else {
            None
        };

        // Execute script with stdin, arguments, and environment
        let mut script_params = serde_json::json!({
            "script": script_code,
            "arguments": arguments
        });

        if let (Some(stdin_val), Some(obj)) = (stdin, script_params.as_object_mut()) {
            obj.insert("stdin".to_string(), serde_json::Value::String(stdin_val));
        }

        if let (Some(env_val), Some(obj)) = (environment, script_params.as_object_mut()) {
            obj.insert("environment".to_string(), env_val);
        }

        // Create streamer for real-time output streaming (before execution)
        let task_index = ctx.state.task_index.unwrap_or(0);
        let streamer = TaskOutputStreamer::new(task_name.to_string(), task_index);

        // Pass streamer directly to executor for real-time streaming
        let script_result = executor
            .exec(task_name, &script_params, ctx, Some(streamer))
            .await?;

        // Output has already been streamed in real-time by the executor!
        // Mark it as streamed so we don't print it again
        let mut result_with_marker = script_result.clone();
        if let Some(obj) = result_with_marker.as_object_mut() {
            obj.insert("__streamed".to_string(), serde_json::Value::Bool(true));
        }

        result_with_marker
    } else if let Some(shell) = run_task.run.shell.as_ref() {
        // Shell command execution
        let command = &shell.command;
        let args = shell.arguments.as_deref().unwrap_or(&[]);

        // Evaluate arguments against current context
        let current_data = ctx.state.data.read().await.clone();
        let evaluated_args: Vec<String> = args
            .iter()
            .map(|arg| {
                // Try to evaluate as expression, fall back to literal string
                match crate::expressions::evaluate_value_with_input(
                    &serde_json::Value::String(arg.clone()),
                    &current_data,
                    &ctx.metadata.initial_input,
                ) {
                    Ok(serde_json::Value::String(s)) => s,
                    _ => arg.clone(),
                }
            })
            .collect();

        // Create streamer for color-coded output
        let task_index = ctx.state.task_index.unwrap_or(0);
        let streamer = TaskOutputStreamer::new(task_name.to_string(), task_index);

        // Execute shell command with piped stdout/stderr for streaming
        let child = tokio::process::Command::new(command)
            .args(&evaluated_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::TaskExecution {
                message: format!("Failed to execute command '{command}': {e}"),
            })?;

        // Stream output in real-time
        let (stdout, stderr, exit_code) =
            streamer
                .stream_process_output(child)
                .await
                .map_err(|e| Error::TaskExecution {
                    message: format!("Failed to stream command output: {e}"),
                })?;

        // Check exit status
        if exit_code != 0 {
            return Err(Error::TaskExecution {
                message: format!(
                    "Command '{command}' failed with exit code {exit_code}\nstdout: {stdout}\nstderr: {stderr}"
                ),
            });
        }

        // Return stdout and stderr as result
        serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code
        })
    } else if let Some(container) = run_task.run.container.as_ref() {
        // Container execution using provider abstraction
        let image = &container.image;
        let command = container.command.as_deref().unwrap_or("sh");

        // Evaluate arguments against current context
        let current_data = ctx.state.data.read().await.clone();
        let evaluated_args: Vec<String> = if let Some(args) = container.arguments.as_ref() {
            args.iter()
                .map(|arg| {
                    // Try to evaluate as expression, fall back to literal string
                    match crate::expressions::evaluate_value_with_input(
                        &serde_json::Value::String(arg.clone()),
                        &current_data,
                        &ctx.metadata.initial_input,
                    ) {
                        Ok(serde_json::Value::String(s)) => s,
                        _ => arg.clone(),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        // Evaluate stdin if provided
        let stdin_data = if let Some(stdin_str) = container.stdin.as_ref() {
            let evaluated = crate::expressions::evaluate_value_with_input(
                &serde_json::Value::String(stdin_str.clone()),
                &current_data,
                &ctx.metadata.initial_input,
            )?;
            evaluated.as_str().map(String::from)
        } else {
            None
        };

        // Build the command to execute in the container
        // Format: sh -c "command" -- arg1 arg2 ...
        // The -- acts as $0, and subsequent args become $1, $2, etc.
        let mut cmd_with_args = vec![String::from("sh"), String::from("-c"), command.to_string()];

        // Add -- as $0, followed by actual arguments
        cmd_with_args.push(String::from("--"));
        cmd_with_args.extend(evaluated_args);

        // Evaluate environment variables if provided
        let environment = if let Some(env) = container.environment.as_ref() {
            let mut evaluated_env = std::collections::HashMap::new();
            for (key, value) in env {
                let evaluated = crate::expressions::evaluate_value_with_input(
                    &serde_json::Value::String(value.clone()),
                    &current_data,
                    &ctx.metadata.initial_input,
                )?;
                // Convert evaluated value to string - handles strings, numbers, bools, etc.
                let value_str = match evaluated {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Null => String::from("null"),
                    _ => evaluated.to_string(), // Arrays and objects as JSON
                };
                evaluated_env.insert(key.clone(), value_str);
            }
            Some(evaluated_env)
        } else {
            None
        };

        // Evaluate volumes if provided
        let volumes = if let Some(vols) = container.volumes.as_ref() {
            let mut evaluated_vols = std::collections::HashMap::new();
            for (key, value) in vols {
                // Evaluate both host path and container path for expressions
                let evaluated_key = crate::expressions::evaluate_value_with_input(
                    &serde_json::Value::String(key.clone()),
                    &current_data,
                    &ctx.metadata.initial_input,
                )?;
                let evaluated_value = crate::expressions::evaluate_value_with_input(
                    &serde_json::Value::String(value.clone()),
                    &current_data,
                    &ctx.metadata.initial_input,
                )?;
                if let (Some(host_path), Some(container_path)) =
                    (evaluated_key.as_str(), evaluated_value.as_str())
                {
                    evaluated_vols.insert(host_path.to_string(), container_path.to_string());
                }
            }
            Some(evaluated_vols)
        } else {
            None
        };

        // Ports don't need expression evaluation (they're numbers)
        let ports = container.ports.clone();

        // Create container provider (Docker for now, could be configurable later)
        let provider = DockerProvider::new().map_err(|e| Error::TaskExecution {
            message: format!("Failed to create container provider: {e}"),
        })?;

        // Execute container
        let config = ContainerConfig {
            image: image.clone(),
            command: cmd_with_args,
            stdin: stdin_data,
            environment,
            working_dir: None, // TODO: Add working directory support if spec adds it
            volumes,
            ports,
        };

        let result = provider
            .execute(config)
            .await
            .map_err(|e| Error::TaskExecution {
                message: format!("Container execution failed: {e}"),
            })?;

        // Check exit status
        if result.exit_code != 0 {
            return Err(Error::TaskExecution {
                message: format!(
                    "Container '{image}' failed with exit code {}\nstdout: {}\nstderr: {}",
                    result.exit_code, result.stdout, result.stderr
                ),
            });
        }

        // Return stdout and stderr as result
        serde_json::json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code
        })
    } else {
        // Other run types not yet implemented
        serde_json::json!({})
    };

    let cache_entry = CacheEntry {
        key: cache_key.clone(),
        inputs: evaluated_params,
        output: result.clone(),
        timestamp: Utc::now(),
    };
    ctx.services.cache.set(cache_entry).await?;

    Ok(result)
}
