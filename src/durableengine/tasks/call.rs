use chrono::Utc;
use std::collections::HashMap;

use crate::cache::{CacheEntry, compute_cache_key};
use crate::context::Context;
use crate::output;
use crate::workflow::WorkflowEvent;

use super::super::{DurableEngine, Result};

/// Execute a Call task - invokes functions (user-defined, catalog, or built-in protocols)
pub async fn exec_call_task(
    engine: &DurableEngine,
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
    let function_result =
        if let Some(function_def) = ctx
            .workflow
            .use_
            .as_ref()
            .and_then(|use_| use_.functions.as_ref())
            .and_then(|funcs| funcs.get(function_name))
        {
            // User-defined function
            use serverless_workflow_core::models::task::TaskDefinition;
            let (call_type, func_params) = match function_def {
                TaskDefinition::Call(call_def) => {
                    (&call_def.call, call_def.with.clone().unwrap_or_default())
                }
                _ => {
                    return Err(super::super::Error::Configuration {
                        message: format!("Function {function_name} is not a call task"),
                    });
                }
            };
            let mut merged_params = func_params;
            merged_params.extend(evaluated_with_params.clone());

            let executor = engine.executors.get(call_type.as_str()).ok_or(
                super::super::Error::TaskExecution {
                    message: format!("No executor for call type: {call_type}"),
                },
            )?;

            let final_params = serde_json::to_value(&merged_params)?;
            executor.exec(task_name, &final_params, ctx).await?
        } else if let Some(catalog_result) = engine
            .try_load_catalog_function(function_name, &evaluated_with_params, ctx)
            .await?
        {
            // Catalog function - execute as nested workflow
            catalog_result
        } else {
            // Built-in protocol
            let executor = engine.executors.get(function_name.as_str()).ok_or(
                super::super::Error::TaskExecution {
                    message: format!("No executor for call type: {function_name}"),
                },
            )?;

            let final_params = serde_json::to_value(&evaluated_with_params)?;
            executor.exec(task_name, &final_params, ctx).await?
        };

    let mut result = function_result;

    // Apply output filtering if specified
    if let Some(output_config) = &call_task.common.output {
        if let Some(as_expr) = &output_config.as_
            && let Some(expr_str) = as_expr.as_str()
        {
            // Evaluate the jq expression on the result with access to $input
            // $input represents the task input (previous task's output for sequential tasks)
            let task_input = ctx.task_input.read().await.clone();
            result = crate::expressions::evaluate_jq_expression_with_context(
                expr_str,
                &result,
                &task_input,
            )?;
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
