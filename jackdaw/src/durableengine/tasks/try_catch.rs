use crate::context::Context;

use super::super::{DurableEngine, Result};

/// Execute a Try task - error handling with catch blocks
// If it's about too many arguments:
pub async fn exec_try_task(
    engine: &DurableEngine,
    task_name: &str,
    try_task: &crate::dsl::task::TryTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Execute the tasks in the try block
    let mut last_result = serde_json::Value::Null;

    for entry in &try_task.try_.entries {
        for (subtask_name, subtask) in entry {
            tracing::debug!("Executing try subtask: {subtask_name}");

            // Box the async call to avoid infinite recursion
            let exec_future = engine.exec_task(subtask_name, subtask, ctx);
            match Box::pin(exec_future).await {
                Ok(result) => {
                    // Update task_input for the next subtask
                    *ctx.state.task_input.write().await = result.clone();

                    // Handle export.as for subtasks (same logic as main execution loop)
                    super::super::export::apply_export_to_context(subtask, &result, ctx).await?;

                    last_result = result;
                }
                Err(e) => {
                    // An error occurred - check if it should be caught

                    // Try to parse the error as JSON to check if it matches the filter
                    // The error might be wrapped in "Executor error: Execution error: {json}"
                    let error_str = e.to_string();
                    let error_obj: serde_json::Value =
                        // First try to parse the whole string as JSON
                        if let Ok(parsed) = serde_json::from_str(&error_str) {
                            parsed
                        } else {
                            // Try to extract JSON from wrapped error messages
                            // Look for patterns like "Executor error: Execution error: {json}"
                            let json_start = error_str.find('{');
                            let json_end = error_str.rfind('}');

                            if let (Some(start), Some(end)) = (json_start, json_end) {
                                if let Ok(parsed) = serde_json::from_str(&error_str[start..=end]) {
                                    parsed
                                } else {
                                    // If extraction failed, create a generic error object
                                    serde_json::json!({
                                        "type": "https://open-workflow-specification.org/spec/1.0.0/errors/runtime",
                                        "status": 500,
                                        "title": "Runtime Error",
                                        "detail": error_str,
                                        "instance": format!("/do/0/{}/try/0/{}", task_name, subtask_name)
                                    })
                                }
                            } else {
                                // No JSON found, create a generic error object
                                serde_json::json!({
                                    "type": "https://open-workflow-specification.org/spec/1.0.0/errors/runtime",
                                    "status": 500,
                                    "title": "Runtime Error",
                                    "detail": error_str,
                                    "instance": format!("/do/0/{}/try/0/{}", task_name, subtask_name)
                                })
                            }
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

                                    // Update task_input for the next subtask
                                    *ctx.state.task_input.write().await = catch_result.clone();

                                    // Handle export.as for catch handler subtasks
                                    super::super::export::apply_export_to_context(
                                        catch_task,
                                        &catch_result,
                                        ctx,
                                    )
                                    .await?;

                                    last_result = catch_result;
                                }
                            }
                        }

                        // Try task returns the last catch handler result
                        return Ok(last_result);
                    }
                    // Error doesn't match the filter, propagate it
                    return Err(e);
                }
            }
        }
    }

    Ok(last_result)
}

/// Check if an error should be caught based on the catch definition
fn should_catch_error(
    error: &serde_json::Value,
    catch_def: &crate::dsl::task::ErrorCatcherDefinition,
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
                // Compare values - need to handle different types. An error type matches in any of its spellings.
                let matches = if key == "type" {
                    error_types_match(expected_value, actual)
                } else {
                    values_match(expected_value, actual)
                };
                if !matches {
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

/// Where the spec puts its standard error types (the DSL reference's "Standard Error Types").
const ERROR_TYPES: &str = "https://open-workflow-specification.org/spec/1.0.0/errors/";

/// Older spellings of the standard error types: the spec 1.0.0 original, the form jackdaw 0.1 raised, and the form
/// the specification's CTK still filters on (open-workflow-specification/specification#1199).
const LEGACY_ERROR_TYPES: [&str; 3] = [
    "https://serverlessworkflow.io/spec/1.0.0/errors/",
    "https://serverlessworkflow.io/dsl/errors/types/",
    "https://open-workflow-specification.org/dsl/errors/types/",
];

/// A standard error type in the spelling the spec gives it now; any other type as it is.
fn canonical_error_type(uri: &str) -> std::borrow::Cow<'_, str> {
    LEGACY_ERROR_TYPES
        .iter()
        .find_map(|prefix| uri.strip_prefix(prefix))
        .map_or(std::borrow::Cow::Borrowed(uri), |name| {
            std::borrow::Cow::Owned(format!("{ERROR_TYPES}{name}"))
        })
}

/// Compare two error types, treating the older spellings of a standard type as that type
fn error_types_match(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::String(e), serde_json::Value::String(a)) => {
            canonical_error_type(e) == canonical_error_type(a)
        }
        _ => values_match(expected, actual),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn standard_error_types_match_in_every_spelling() {
        let current =
            json!("https://open-workflow-specification.org/spec/1.0.0/errors/communication");
        for legacy in [
            "https://serverlessworkflow.io/spec/1.0.0/errors/communication",
            "https://serverlessworkflow.io/dsl/errors/types/communication",
            "https://open-workflow-specification.org/dsl/errors/types/communication",
        ] {
            assert!(error_types_match(&json!(legacy), &current), "{legacy}");
            assert!(error_types_match(&current, &json!(legacy)), "{legacy}");
        }
    }

    #[test]
    fn different_error_types_do_not_match() {
        assert!(!error_types_match(
            &json!("https://open-workflow-specification.org/dsl/errors/types/communication"),
            &json!("https://open-workflow-specification.org/spec/1.0.0/errors/runtime"),
        ));
        assert!(!error_types_match(
            &json!("https://example.com/errors/communication"),
            &json!("https://open-workflow-specification.org/spec/1.0.0/errors/communication"),
        ));
    }

    #[test]
    fn custom_error_types_match_exactly() {
        let custom = json!("https://example.com/errors/out-of-stock");
        assert!(error_types_match(&custom, &custom));
        assert_eq!(
            canonical_error_type("https://example.com/errors/out-of-stock"),
            "https://example.com/errors/out-of-stock"
        );
    }
}
