use crate::context::Context;
use crate::task_ext::TaskDefinitionExt;
use serverless_workflow_core::models::task::TaskDefinition;

/// Apply export configuration to update context based on task result
///
/// # Arguments
/// * `task` - The task definition containing export config
/// * `result` - The task execution result
/// * `ctx` - The execution context to update
///
/// # Errors
/// Returns an error if expression evaluation fails
pub async fn apply_export_to_context(
    task: &TaskDefinition,
    result: &serde_json::Value,
    ctx: &Context,
) -> Result<(), crate::expressions::Error> {
    let export_config = task.export();

    if let Some(export_def) = export_config
        && let Some(export_expr) = &export_def.as_
        && let Some(expr_str) = export_expr.as_str()
    {
        // Evaluate export.as expression on the transformed task output
        // The result becomes the new context
        let new_context = crate::expressions::evaluate_expression(expr_str, result)?;
        *ctx.state.data.write().await = new_context;
        return Ok(());
    }

    // No explicit export.as - apply default behavior
    // Default: merge the transformed task output into the existing context
    let mut current_context = ctx.state.data.write().await;
    if let serde_json::Value::Object(result_obj) = result {
        if let Some(context_obj) = (*current_context).as_object_mut() {
            for (key, value) in result_obj {
                context_obj.insert(key.clone(), value.clone());
            }
        }
    } else {
        // If result is not an object, we cannot merge - replace the context entirely
        *current_context = result.clone();
    }

    Ok(())
}
