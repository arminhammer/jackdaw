use crate::context::Context;

use super::super::{DurableEngine, Result};

/// Execute a Switch task - conditional branching based on evaluated expressions
pub async fn exec_switch_task(
    _engine: &DurableEngine,
    _task_name: &str,
    switch_task: &serverless_workflow_core::models::task::SwitchTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Get current context data
    let current_data = ctx.data.read().await.clone();

    // Evaluate each case in order
    for entry in &switch_task.switch.entries {
        for (_case_name, case_def) in entry {
            // If there's a 'when' condition, evaluate it
            let matches = if let Some(when_expr) = &case_def.when {
                // Evaluate the condition expression
                let result = crate::expressions::evaluate_jq(when_expr, &current_data)?;

                // Check if the result is truthy
                let matches = match result {
                    serde_json::Value::Bool(b) => b,
                    serde_json::Value::Null => false,
                    _ => true, // Non-null, non-bool values are truthy
                };

                matches
            } else {
                // No 'when' condition means this is a default case
                true
            };

            if matches {
                // Set the next task to the matched case's 'then' target
                if let Some(then_target) = &case_def.then {
                    *ctx.next_task.write().await = Some(then_target.clone());
                }
                // The switch task doesn't execute anything itself
                // It just evaluates conditions and the graph will follow the 'then' transition
                // Return empty result since the actual work is done by the transitioned-to task
                return Ok(serde_json::json!({}));
            }
        }
    }

    // No cases matched - check if there's a common 'then' transition
    if let Some(then_target) = &switch_task.common.then {
        *ctx.next_task.write().await = Some(then_target.clone());
    }
    Ok(serde_json::json!({}))
}