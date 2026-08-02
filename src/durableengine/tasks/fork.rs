use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::context::Context;

use super::super::{DurableEngine, Error, Result};

/// Give a fork branch its own independent copy of the mutable execution
/// state (context data, task-input tracking, current/next task pointers).
///
/// `Context::clone()` is a shallow `#[derive(Clone)]`: `state.data` and its
/// siblings are `Arc<RwLock<_>>`, so a plain clone shares the *same*
/// underlying storage. Without this, concurrently running branches read and
/// write the same `RwLock<Value>` — one branch's output can race with, or
/// silently overwrite, another's, and every branch observes every sibling's
/// writes mid-flight instead of only its own. Cancellation/suspension flags
/// are intentionally left shared: those are whole-workflow signals, not
/// per-branch data, so all branches should still see them.
async fn isolate_branch_context(ctx: &Context) -> Context {
    let mut branch_ctx = ctx.clone();
    let data_snapshot = ctx.state.data.read().await.clone();
    let task_input_snapshot = ctx.state.task_input.read().await.clone();
    branch_ctx.state.data = Arc::new(RwLock::new(data_snapshot));
    branch_ctx.state.task_input = Arc::new(RwLock::new(task_input_snapshot));
    branch_ctx.state.current_task = Arc::new(RwLock::new(String::new()));
    branch_ctx.state.next_task = Arc::new(RwLock::new(None));
    branch_ctx
}

/// Execute a Fork task - parallel execution of branches with optional compete mode
///
/// Each branch's own transformed output lands under its branch name in the
/// returned map (`{branch_name: branch_result}`) — the same contract as
/// before the isolation fix. `data`/`export.as` is a separate, opt-in
/// mechanism the engine's main loop resets around every task (see
/// `durableengine.rs`'s `original_context` restore), so a branch can't push
/// values into the parent context that way regardless of isolation; a
/// caller that wants a flat merge of every branch's contribution sets the
/// fork task's own `output.as`, e.g. `${ [.[]] | add }`.
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
        let engine = Arc::new(engine);

        let mut branch_index = 0;
        for entry in &fork_task.fork.branches.entries {
            for (branch_name, branch_task) in entry {
                let branch_name = branch_name.clone();
                let branch_task = branch_task.clone();
                let mut branch_ctx = isolate_branch_context(ctx).await;
                branch_ctx.state.task_index = Some(branch_index);
                let engine = Arc::clone(&engine);

                let future = Box::pin(async move {
                    let result = engine
                        .exec_task(&branch_name, &branch_task, &branch_ctx)
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
        let engine = Arc::new(engine);

        let mut branch_index = 0;
        for entry in &fork_task.fork.branches.entries {
            for (branch_name, branch_task) in entry {
                let branch_name = branch_name.clone();
                let branch_task = branch_task.clone();
                let mut branch_ctx = isolate_branch_context(ctx).await;
                branch_ctx.state.task_index = Some(branch_index);
                let engine = Arc::clone(&engine);

                let future = async move {
                    let result = engine
                        .exec_task(&branch_name, &branch_task, &branch_ctx)
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use serverless_workflow_core::models::workflow::WorkflowDefinition;

    use crate::builder::DurableEngineBuilder;

    const TIMEOUT: Duration = Duration::from_secs(10);

    /// Fork is the workflow's only task, so its own raw `{branch: result}`
    /// map (see `exec_fork_task`'s return value) *is* the workflow output —
    /// each branch's own `seen` reflects only what that branch itself wrote.
    fn isolation_workflow(name: &str) -> WorkflowDefinition {
        let yaml = format!(
            r#"
document:
  dsl: '1.0.2'
  namespace: test
  name: {name}
  version: '0.1.0'
do:
  - parallel:
      fork:
        compete: false
        branches:
          - branch-a:
              set: '${{ . + {{"seen": "A"}} }}'
          - branch-b:
              set: '${{ . + {{"seen": "B"}} }}'
"#
        );
        serde_yaml::from_str(&yaml).unwrap()
    }

    /// No task type but `run` implements `output.as` itself, and
    /// `exec_task`'s dispatcher applies none of it generically — so a caller
    /// flattens the fork's branch-keyed map by wrapping it in a `do` block
    /// ending with a plain `set`, the same pattern `switch()`'s Python
    /// compiler uses for its own join step. That join must read `$input`
    /// (the fork's own raw result), not `.`: by the time it runs, `exec_do_task`
    /// has already merged the fork's branch-keyed result into `.` (the
    /// default no-`export.as` behavior) *alongside* whatever unrelated data
    /// was already there — `seed`, here, standing in for a Session's seed
    /// input fields. `[.[]] | add` over `.` would try to JQ-`+` a plain
    /// string together with the branch objects and fail.
    fn merge_workflow(name: &str) -> WorkflowDefinition {
        let yaml = format!(
            r#"
document:
  dsl: '1.0.2'
  namespace: test
  name: {name}
  version: '0.1.0'
do:
  - wrapper:
      do:
        - parallel:
            fork:
              compete: false
              branches:
                - branch-a:
                    set: '${{ . + {{"from_a": "A", "shared": "A"}} }}'
                - branch-b:
                    set: '${{ . + {{"from_b": "B", "shared": "B"}} }}'
        - merge:
            set: '${{ [$input[]] | add }}'
"#
        );
        serde_yaml::from_str(&yaml).unwrap()
    }

    /// Branches must not see each other's writes: a shared-storage race (the
    /// bug this test guards against) would let one branch's `set` clobber or
    /// leak into the other's own result.
    #[tokio::test]
    async fn fork_branches_get_isolated_context() {
        let engine = DurableEngineBuilder::new().build().unwrap();
        let handle = engine
            .execute(isolation_workflow("fork-isolation"), json!({}))
            .await
            .unwrap();
        let result = handle.wait_for_completion(TIMEOUT).await.unwrap();

        assert_eq!(result["branch-a"].get("seen"), Some(&json!("A")));
        assert_eq!(result["branch-b"].get("seen"), Some(&json!("B")));
    }

    /// Repeats the same workflow enough times that, before the isolation
    /// fix, cooperative scheduling across the shared `RwLock<Value>` would
    /// eventually interleave badly and drop or swap a branch's contribution.
    #[tokio::test]
    async fn fork_isolation_is_stable_across_repeated_runs() {
        let engine = DurableEngineBuilder::new().build().unwrap();
        for i in 0..25 {
            let handle = engine
                .execute(isolation_workflow(&format!("fork-isolation-{i}")), json!({}))
                .await
                .unwrap();
            let result = handle.wait_for_completion(TIMEOUT).await.unwrap();
            assert_eq!(result["branch-a"].get("seen"), Some(&json!("A")), "run {i}");
            assert_eq!(result["branch-b"].get("seen"), Some(&json!("B")), "run {i}");
        }
    }

    /// A caller-supplied join flattening the branch-keyed map must see both
    /// branches' non-conflicting keys, with the conflicting key resolving
    /// deterministically to the last-declared branch — proving the
    /// isolation fix didn't corrupt either branch's own contribution. The
    /// non-empty, non-object-only seed input (`seed: "unrelated"`) is what
    /// makes this test meaningful: it reproduces the pollution `.` picks up
    /// once the fork's result lands in context, which a naive `[.[]] | add`
    /// over `.` chokes on.
    #[tokio::test]
    async fn fork_merges_branch_data_into_parent_context() {
        let engine = DurableEngineBuilder::new().build().unwrap();
        let handle = engine
            .execute(merge_workflow("fork-merge"), json!({"seed": "unrelated"}))
            .await
            .unwrap();
        let result = handle.wait_for_completion(TIMEOUT).await.unwrap();

        assert_eq!(result.get("from_a"), Some(&json!("A")));
        assert_eq!(result.get("from_b"), Some(&json!("B")));
        assert_eq!(result.get("shared"), Some(&json!("B")));
    }
}
