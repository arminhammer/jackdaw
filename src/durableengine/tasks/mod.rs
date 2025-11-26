use serverless_workflow_core::models::task::TaskDefinition;
use std::collections::HashMap;

use crate::context::Context;
use crate::output;

use super::{DurableEngine, Result};

// Submodules for individual task types
mod call;
mod emit;
mod for_loop;
mod fork;
mod raise;
mod run;
mod switch;
mod try_catch;

// Re-export task execution methods
pub use call::exec_call_task;
pub use emit::exec_emit_task;
pub use for_loop::exec_for_task;
pub use fork::exec_fork_task;
pub use raise::exec_raise_task;
pub use run::exec_run_task;
pub use switch::exec_switch_task;
pub use try_catch::exec_try_task;

impl DurableEngine {
    /// Main task execution dispatcher
    pub(super) async fn exec_task(
        &self,
        task_name: &str,
        task: &TaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Determine task type for display
        let task_type = match task {
            TaskDefinition::Call(_) => "Call",
            TaskDefinition::Set(_) => "Set",
            TaskDefinition::Fork(_) => "Fork",
            TaskDefinition::Run(_) => "Run",
            TaskDefinition::Do(_) => "Do",
            TaskDefinition::For(_) => "For",
            TaskDefinition::Switch(_) => "Switch",
            TaskDefinition::Try(_) => "Try",
            TaskDefinition::Emit(_) => "Emit",
            TaskDefinition::Raise(_) => "Raise",
            TaskDefinition::Wait(_) => "Wait",
            TaskDefinition::Listen(_) => "Listen",
        };

        // Format task start
        output::format_task_start(task_name, task_type);

        // Show current context
        let current_context = ctx.data.read().await.clone();
        output::format_task_context(&current_context);

        // Apply input filtering if specified
        let _has_input_filter = self.apply_input_filter(task, ctx).await?;

        // Show input after filtering
        let input_data = ctx.data.read().await.clone();
        output::format_task_input(&input_data);

        // Execute the task
        let result = match task {
            TaskDefinition::Call(call_task) => {
                exec_call_task(self, task_name, call_task, ctx).await
            }
            TaskDefinition::Set(set_task) => exec_set_task(self, task_name, set_task, ctx).await,
            TaskDefinition::Fork(fork_task) => {
                exec_fork_task(self, task_name, fork_task, ctx).await
            }
            TaskDefinition::Run(run_task) => exec_run_task(self, task_name, run_task, ctx).await,
            TaskDefinition::Do(do_task) => exec_do_task(self, task_name, do_task, ctx).await,
            TaskDefinition::For(for_task) => exec_for_task(self, task_name, for_task, ctx).await,
            TaskDefinition::Switch(switch_task) => {
                exec_switch_task(self, task_name, switch_task, ctx).await
            }
            TaskDefinition::Raise(raise_task) => {
                exec_raise_task(self, task_name, raise_task, ctx).await
            }
            TaskDefinition::Try(try_task) => exec_try_task(self, task_name, try_task, ctx).await,
            TaskDefinition::Emit(emit_task) => {
                exec_emit_task(self, task_name, emit_task, ctx).await
            }
            TaskDefinition::Listen(listen_task) => {
                exec_listen_task(self, task_name, listen_task, ctx).await
            }
            _ => {
                println!("  Task type not yet implemented, returning empty result");
                Ok(serde_json::json!({}))
            }
        };

        // Note: We don't restore the original context after input filtering
        // because task outputs (via ctx.merge) should be preserved
        result
    }

    /// Apply input filter to task
    pub(super) async fn apply_input_filter(
        &self,
        task: &TaskDefinition,
        ctx: &Context,
    ) -> Result<bool> {
        // Get the common task fields to check for input.from
        let input_config = match task {
            TaskDefinition::Call(t) => t.common.input.as_ref(),
            TaskDefinition::Set(t) => t.common.input.as_ref(),
            TaskDefinition::Fork(t) => t.common.input.as_ref(),
            TaskDefinition::Run(t) => t.common.input.as_ref(),
            TaskDefinition::Do(t) => t.common.input.as_ref(),
            TaskDefinition::For(t) => t.common.input.as_ref(),
            TaskDefinition::Switch(t) => t.common.input.as_ref(),
            TaskDefinition::Try(t) => t.common.input.as_ref(),
            TaskDefinition::Emit(t) => t.common.input.as_ref(),
            TaskDefinition::Raise(t) => t.common.input.as_ref(),
            _ => None,
        };

        if let Some(input) = input_config {
            if let Some(from_expr) = &input.from {
                if let Some(expr_str) = from_expr.as_str() {
                    let current_data = ctx.data.read().await.clone();
                    // Input filtering uses jq expressions directly (not wrapped in ${ })
                    let filtered = crate::expressions::evaluate_jq(
                        expr_str,
                        &current_data,
                    )?;
                    *ctx.data.write().await = filtered;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}

/// Execute a Set task - sets variables in the context
async fn exec_set_task(
    _engine: &DurableEngine,
    _task_name: &str,
    set_task: &serverless_workflow_core::models::task::SetTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Get current context data for expression evaluation
    let current_data = ctx.data.read().await.clone();

    for (key, value) in set_task.set.iter() {
        // Evaluate expressions in the value using current context and initial input
        let evaluated_value = crate::expressions::evaluate_value_with_input(
            value,
            &current_data,
            &ctx.initial_input,
        )?;
        ctx.merge(key, evaluated_value.clone()).await;
    }
    Ok(serde_json::to_value(&set_task.set)?)
}

/// Execute a Do task - sequential execution of subtasks
async fn exec_do_task(
    engine: &DurableEngine,
    _task_name: &str,
    do_task: &serverless_workflow_core::models::task::DoTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    let mut results = serde_json::Map::new();

    // Execute subtasks sequentially in order
    for entry in &do_task.do_.entries {
        for (subtask_name, subtask) in entry {
            // Box the recursive call to avoid infinite sized future
            let result = Box::pin(engine.exec_task(subtask_name, subtask, ctx)).await?;
            results.insert(subtask_name.clone(), result);
        }
    }

    Ok(serde_json::Value::Object(results))
}

/// Execute a Listen task - listeners are initialized at workflow startup
async fn exec_listen_task(
    _engine: &DurableEngine,
    _task_name: &str,
    _listen_task: &serverless_workflow_core::models::task::ListenTaskDefinition,
    _ctx: &Context,
) -> Result<serde_json::Value> {
    // Listen tasks are now initialized at workflow startup via initialize_listeners()
    // This method is kept for compatibility but does nothing during execution
    // The listener is already running and will continue to run until workflow completes
    Ok(serde_json::json!({"status": "already_listening"}))
}
