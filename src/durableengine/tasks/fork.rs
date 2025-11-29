use std::collections::HashMap;

use crate::context::Context;

use super::super::{DurableEngine, Error, Result};

/// Execute a Fork task - parallel execution of branches with optional compete mode
pub async fn exec_fork_task(
    engine: &DurableEngine,
    _task_name: &str,
    fork_task: &serverless_workflow_core::models::task::ForkTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Check if compete mode is enabled - use different future types
    if fork_task.fork.compete {
        // In compete mode, use boxed futures for select_all (requires Unpin)
        let mut branch_futures = Vec::new();

        let mut branch_index = 0;
        for entry in &fork_task.fork.branches.entries {
            for (branch_name, branch_task) in entry {
                let branch_name = branch_name.clone();
                let branch_task = branch_task.clone();
                let mut ctx = ctx.clone();
                ctx.task_index = Some(branch_index);
                let engine = engine as *const DurableEngine;

                let future = Box::pin(async move {
                    let engine_ref = unsafe { &*engine };
                    let result = engine_ref
                        .exec_task(&branch_name, &branch_task, &ctx)
                        .await?;
                    Ok::<_, Error>((branch_name, result))
                });
                branch_futures.push(future);
                branch_index += 1;
            }
        }

        if !branch_futures.is_empty() {
            let (result, _index, _remaining) = futures::future::select_all(branch_futures).await;
            let (_branch_name, branch_result) = result?;
            // In compete mode, return only the winning branch's result
            return Ok(branch_result);
        }

        // No branches - return empty object
        Ok(serde_json::json!({}))
    } else {
        // In normal mode, plain futures work fine with join_all
        let mut branch_futures = Vec::new();
        let mut results = HashMap::new();

        let mut branch_index = 0;
        for entry in &fork_task.fork.branches.entries {
            for (branch_name, branch_task) in entry {
                let branch_name = branch_name.clone();
                let branch_task = branch_task.clone();
                let mut ctx = ctx.clone();
                ctx.task_index = Some(branch_index);
                let engine = engine as *const DurableEngine;

                let future = async move {
                    let engine_ref = unsafe { &*engine };
                    let result = engine_ref
                        .exec_task(&branch_name, &branch_task, &ctx)
                        .await?;
                    Ok::<_, Error>((branch_name, result))
                };
                branch_futures.push(future);
                branch_index += 1;
            }
        }

        let branch_results = futures::future::join_all(branch_futures).await;

        for result in branch_results {
            let (branch_name, branch_result) = result?;
            results.insert(branch_name, branch_result);
        }

        Ok(serde_json::to_value(&results)?)
    }
}
