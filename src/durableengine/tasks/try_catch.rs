use crate::context::Context;

use super::super::{DurableEngine, Result};

/// Execute a Try task - error handling with catch blocks
pub async fn exec_try_task(
    engine: &DurableEngine,
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
            let exec_future = engine.exec_task(subtask_name, subtask, ctx);
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
                    let should_catch = should_catch_error(&error_obj, &try_task.catch);

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
                                        engine.exec_task(catch_task_name, catch_task, ctx);
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

/// Check if an error should be caught based on the catch definition
fn should_catch_error(
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

/// Compare two JSON values for equality, handling different numeric types
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