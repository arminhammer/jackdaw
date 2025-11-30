use crate::context::Context;

use super::super::{DurableEngine, Error, Result};

/// Execute a For task - iterates over a collection and executes tasks for each item
pub async fn exec_for_task(
    engine: &DurableEngine,
    _task_name: &str,
    for_task: &serverless_workflow_core::models::task::ForTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Get current context data
    let current_data = ctx.data.read().await.clone();

    // Evaluate the 'in' expression to get the collection to iterate over
    let collection_expr = &for_task.for_.in_;
    let collection = crate::expressions::evaluate_jq(collection_expr, &current_data)?;

    // Get the collection as an array
    let items = collection.as_array().ok_or(Error::TaskExecution {
        message: format!(
            "For loop 'in' expression must evaluate to an array, got: {:?}",
            collection
        ),
    })?;

    // Get the iteration variable name (e.g., "color")
    let item_var = &for_task.for_.each;

    // Get the index variable name (defaults to "index" if not specified)
    let index_var = for_task.for_.at.as_deref().unwrap_or("index");

    let mut last_result = serde_json::Value::Null;

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
                let result = Box::pin(engine.exec_task(subtask_name, subtask, ctx)).await?;

                // Update task_input for the next subtask
                *ctx.task_input.write().await = result.clone();

                // Handle export.as for subtasks (same logic as main execution loop)
                use serverless_workflow_core::models::task::TaskDefinition;
                let export_config = match subtask {
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
                    if let Some(export_expr) = &export_def.as_ {
                        if let Some(expr_str) = export_expr.as_str() {
                            let new_context =
                                crate::expressions::evaluate_expression(expr_str, &result)?;
                            *ctx.data.write().await = new_context;
                        }
                    }
                } else {
                    // No explicit export.as - apply default behavior (merge into context)
                    let mut current_context = ctx.data.write().await;
                    if let serde_json::Value::Object(result_obj) = &result {
                        if let Some(context_obj) = (*current_context).as_object_mut() {
                            for (key, value) in result_obj {
                                context_obj.insert(key.clone(), value.clone());
                            }
                        }
                    }
                }

                last_result = result;
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

    // For task returns the last subtask's result
    Ok(last_result)
}
