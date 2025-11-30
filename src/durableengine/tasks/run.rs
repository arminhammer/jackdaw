use chrono::Utc;
use std::process::Stdio;

use crate::cache::{CacheEntry, compute_cache_key};
use crate::context::Context;
use crate::output;
use crate::task_output::TaskOutputStreamer;
use crate::workflow::WorkflowEvent;

use super::super::{DurableEngine, Error, Result};

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
    let evaluated_params =
        crate::expressions::evaluate_value_with_input(&params, &current_data, &ctx.metadata.initial_input)?;

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

    ctx.services.persistence
        .save_event(WorkflowEvent::TaskStarted {
            instance_id: ctx.metadata.instance_id.clone(),
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
        // Script execution
        let executor = engine.executors.get("python").ok_or(Error::TaskExecution {
            message: "No python executor found".to_string(),
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
                let file_path = source_uri.strip_prefix("file://").unwrap();
                tokio::fs::read_to_string(file_path)
                    .await
                    .map_err(|e| Error::Io { source: e })?
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
            serde_json::json!({})
        };

        // Execute script with arguments injected as globals
        let script_params = serde_json::json!({
            "script": script_code,
            "arguments": arguments
        });

        executor.exec(task_name, &script_params, ctx).await?
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
    } else {
        // Other run types (container, etc.) not yet implemented
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
