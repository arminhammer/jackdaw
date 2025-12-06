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
    let current_data = ctx.state.data.read().await.clone();

    // Evaluate the 'in' expression to get the collection to iterate over
    let collection_expr = &for_task.for_.in_;
    let collection = crate::expressions::evaluate_jq(collection_expr, &current_data)?;

    // Get the collection as an array
    let items = collection.as_array().ok_or(Error::TaskExecution {
        message: format!("For loop 'in' expression must evaluate to an array, got: {collection:?}"),
    })?;

    // Get the iteration variable name (e.g., "color")
    let item_var = &for_task.for_.each;

    // Get the index variable name (defaults to "index" if not specified)
    let index_var = for_task.for_.at.as_deref().unwrap_or("index");

    let mut last_result = serde_json::Value::Null;

    // Iterate over the collection
    for (index, item) in items.iter().enumerate() {
        // Get current accumulated state (includes updates from previous iterations)
        let accumulated_data = ctx.state.data.read().await.clone();

        // Inject iteration variables into the current state
        let mut iteration_data = accumulated_data;
        if let Some(obj) = iteration_data.as_object_mut() {
            // Store the item and index as variables (without $ prefix, jq will handle $ reference)
            obj.insert(item_var.clone(), item.clone());
            obj.insert(index_var.to_string(), serde_json::json!(index));
        }

        // Update context with iteration variables
        {
            let mut data_guard = ctx.state.data.write().await;
            *data_guard = iteration_data;
        }

        // Execute the do tasks for this iteration
        for entry in &for_task.do_.entries {
            for (subtask_name, subtask) in entry {
                let result = Box::pin(engine.exec_task(subtask_name, subtask, ctx)).await?;

                // Update task_input for the next subtask
                *ctx.state.task_input.write().await = result.clone();

                // Handle export.as for subtasks (same logic as main execution loop)
                super::super::export::apply_export_to_context(subtask, &result, ctx).await?;

                last_result = result;
            }
        }

        // Remove iteration variables but keep accumulated changes
        {
            let mut data_guard = ctx.state.data.write().await;
            if let Some(obj) = data_guard.as_object_mut() {
                obj.remove(item_var);
                obj.remove(index_var);
            }
        }
    }

    // For task returns the last subtask's result
    Ok(last_result)
}
