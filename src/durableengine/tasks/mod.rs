use serverless_workflow_core::models::task::TaskDefinition;

use crate::context::Context;
use crate::output;
use crate::task_ext::TaskDefinitionExt;

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
mod wait;

// Re-export task execution methods
pub use call::exec_call_task;
pub use emit::exec_emit_task;
pub use for_loop::exec_for_task;
pub use fork::exec_fork_task;
pub use raise::exec_raise_task;
pub use run::exec_run_task;
pub use switch::exec_switch_task;
pub use try_catch::exec_try_task;
pub use wait::exec_wait_task;

impl DurableEngine {
    /// Main task execution dispatcher
    pub(super) async fn exec_task(
        &self,
        task_name: &str,
        task: &TaskDefinition,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Check if workflow is cancelled before executing task
        if ctx.is_cancelled().await {
            let reason = ctx.state.cancellation_reason.read().await.clone();

            // Emit task.cancelled.v1 event
            ctx.services
                .persistence
                .save_event(crate::workflow::WorkflowEvent::TaskCancelled {
                    instance_id: ctx.metadata.instance_id.clone(),
                    task_name: task_name.to_string(),
                    reason: reason.clone(),
                    timestamp: chrono::Utc::now(),
                })
                .await?;

            return Err(super::Error::WorkflowExecution {
                message: format!("Workflow cancelled: {}", reason.unwrap_or_else(|| "No reason provided".to_string())),
            });
        }

        // Check if workflow is suspended before executing task
        if ctx.is_suspended().await {
            let reason = ctx.state.suspension_reason.read().await.clone();
            return Err(super::Error::WorkflowExecution {
                message: format!("Workflow suspended: {}", reason.unwrap_or_else(|| "No reason provided".to_string())),
            });
        }

        // Emit task.created.v1 event
        ctx.services
            .persistence
            .save_event(crate::workflow::WorkflowEvent::TaskCreated {
                instance_id: ctx.metadata.instance_id.clone(),
                task_name: task_name.to_string(),
                task_type: task.type_name().to_string(),
                timestamp: chrono::Utc::now(),
            })
            .await?;

        // Emit task.started.v1 event
        ctx.services
            .persistence
            .save_event(crate::workflow::WorkflowEvent::TaskStarted {
                instance_id: ctx.metadata.instance_id.clone(),
                task_name: task_name.to_string(),
                timestamp: chrono::Utc::now(),
            })
            .await?;

        // Format task start
        output::format_task_start(task_name, task.type_name());

        // Show current context
        let current_context = ctx.state.data.read().await.clone();
        output::format_task_context(&current_context);

        // Apply input filtering if specified
        let _has_input_filter = self.apply_input_filter(task, ctx).await?;

        // Show input after filtering
        let input_data = ctx.state.data.read().await.clone();
        output::format_task_input(&input_data);

        // Execute the task
        // Note: We don't restore the original context after input filtering
        // because task outputs (via ctx.merge) should be preserved
        let task_execution_future = async {
            match task {
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
                TaskDefinition::Wait(wait_task) => {
                    exec_wait_task(self, task_name, wait_task, ctx).await
                }
            }
        };

        // Apply task-level timeout if specified
        if let Some(timeout_def) = task.timeout() {
            let timeout_duration = super::timeout::parse_timeout_duration(timeout_def)?;
            
            match tokio::time::timeout(timeout_duration, task_execution_future).await {
                Ok(result) => result,
                Err(_) => {
                    // Task timed out - emit TaskFaulted event
                    ctx.services
                        .persistence
                        .save_event(crate::workflow::WorkflowEvent::TaskFaulted {
                            instance_id: ctx.metadata.instance_id.clone(),
                            task_name: task_name.to_string(),
                            error: format!("Task '{}' timed out after {:?}", task_name, timeout_duration),
                            timestamp: chrono::Utc::now(),
                        })
                        .await?;
                    
                    Err(super::Error::Timeout {
                        message: format!("Task '{}' exceeded timeout of {:?}", task_name, timeout_duration),
                    })
                }
            }
        } else {
            // No timeout specified, execute normally
            task_execution_future.await
        }
    }

    /// Apply input filter to task
    pub(super) async fn apply_input_filter(
        &self,
        task: &TaskDefinition,
        ctx: &Context,
    ) -> Result<bool> {
        if let Some(input) = task.input()
            && let Some(from_expr) = &input.from
            && let Some(expr_str) = from_expr.as_str()
        {
            let current_data = ctx.state.data.read().await.clone();
            // Input filtering can use either:
            // 1. Wrapped expressions: ${ .field } (newer CTK examples)
            // 2. Bare JQ expressions: .field (older examples)
            let filtered = if expr_str.trim().starts_with("${") {
                // Wrapped expression - use evaluate_expression which handles ${ } syntax
                crate::expressions::evaluate_expression(expr_str, &current_data)?
            } else {
                // Bare JQ expression - use evaluate_jq directly
                crate::expressions::evaluate_jq(expr_str, &current_data)?
            };
            *ctx.state.data.write().await = filtered;
            return Ok(true);
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
    use serverless_workflow_core::models::task::SetValue;

    // Get current context data for expression evaluation
    let current_data = ctx.state.data.read().await.clone();

    let task_input = ctx.state.task_input.read().await.clone();

    match &set_task.set {
        SetValue::Map(map) => {
            // Handle map of key-value pairs - evaluate each value
            let mut result_map = serde_json::Map::new();
            for (key, value) in map {
                let evaluated_value = crate::expressions::evaluate_value_with_input(
                    value,
                    &current_data,
                    &task_input,
                )?;
                result_map.insert(key.clone(), evaluated_value);
            }
            Ok(serde_json::Value::Object(result_map))
        }
        SetValue::Expression(expr) => {
            // Handle runtime expression - evaluate it and return the result
            let evaluated_value = crate::expressions::evaluate_expression_with_input(
                expr,
                &current_data,
                &task_input,
            )?;
            Ok(evaluated_value)
        }
    }
}

/// Execute a Do task - sequential execution of subtasks
async fn exec_do_task(
    engine: &DurableEngine,
    _task_name: &str,
    do_task: &serverless_workflow_core::models::task::DoTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    let mut last_result = serde_json::Value::Null;

    // Execute subtasks sequentially in order
    for entry in &do_task.do_.entries {
        for (subtask_name, subtask) in entry {
            // Box the recursive call to avoid infinite sized future
            let result = Box::pin(engine.exec_task(subtask_name, subtask, ctx)).await?;

            // Update task_input for the next subtask
            *ctx.state.task_input.write().await = result.clone();

            // Handle export.as for subtasks (same logic as main execution loop)
            super::export::apply_export_to_context(subtask, &result, ctx).await?;

            last_result = result;
        }
    }

    // Do task returns the last subtask's result
    Ok(last_result)
}

/// Execute a Listen task - listeners are initialized at workflow startup
#[allow(clippy::unnecessary_wraps)]
async fn exec_listen_task(
    _engine: &DurableEngine,
    _task_name: &str,
    listen_task: &serverless_workflow_core::models::task::ListenTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Listen tasks are initialized at workflow startup via initialize_listeners()
    // The listener is already running in the background.
    // According to the DSL spec, if 'until' is specified with eventConsumptionStrategy 'any',
    // we must keep listening until the condition evaluates to true.
    // If until evaluates to false, we block indefinitely to keep the workflow alive.

    let listen_def = &listen_task.listen;
    if let Some(until_box) = &listen_def.to.until {
        use serverless_workflow_core::models::event::OneOfEventConsumptionStrategyDefinitionOrExpression;

        // Check if until is an expression (not a strategy with events)
        if let OneOfEventConsumptionStrategyDefinitionOrExpression::Expression(until_expr) = until_box.as_ref() {
            // Evaluate the until expression
            let current_data = ctx.state.data.read().await.clone();
            let until_value = crate::expressions::evaluate_expression(&until_expr, &current_data)?;

            // If until evaluates to false, block indefinitely
            // This keeps the workflow (and container) alive while background listeners process events
            if let Some(false) = until_value.as_bool() {
                eprintln!("DEBUG: Blocking forever because until = false");
                use tokio::time::Duration;
                loop {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
            } else {
                eprintln!("DEBUG: NOT blocking, until_value.as_bool() = {:?}", until_value.as_bool());
            }
        }
        // Note: If until is a Strategy (not an Expression), we don't block here
        // because the strategy defines events that trigger completion, not a boolean condition
    }

    Ok(serde_json::json!({"status": "already_listening"}))
}
