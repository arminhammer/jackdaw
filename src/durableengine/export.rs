use crate::context::Context;
use serverless_workflow_core::models::output::OutputDataModelDefinition;
use serverless_workflow_core::models::task::TaskDefinition;

/// Extract export configuration from a task definition
pub fn get_task_export(task: &TaskDefinition) -> Option<&OutputDataModelDefinition> {
    match task {
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
    }
}

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
    let export_config = get_task_export(task);

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
