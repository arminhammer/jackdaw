//! Which names a workflow definition calls, at any depth. Used to reject a workflow that calls something nobody
//! provides before any of its tasks run.
use crate::task_ext::call_name;
use serverless_workflow_core::models::map::Map;
use serverless_workflow_core::models::task::TaskDefinition;
use serverless_workflow_core::models::workflow::WorkflowDefinition;

pub(crate) fn called_names(definition: &WorkflowDefinition) -> Vec<String> {
    let mut names = Vec::new();
    collect_map(&definition.do_, &mut names);
    names.sort();
    names.dedup();
    names
}

fn collect_map(tasks: &Map<String, TaskDefinition>, names: &mut Vec<String>) {
    for entry in &tasks.entries {
        for task in entry.values() {
            collect(task, names);
        }
    }
}

fn collect(task: &TaskDefinition, names: &mut Vec<String>) {
    match task {
        TaskDefinition::Call(call) => names.push(call_name(call).to_string()),
        TaskDefinition::Do(do_task) => collect_map(&do_task.do_, names),
        TaskDefinition::For(for_task) => collect_map(&for_task.do_, names),
        TaskDefinition::Fork(fork_task) => collect_map(&fork_task.fork.branches, names),
        TaskDefinition::Try(try_task) => collect_map(&try_task.try_, names),
        TaskDefinition::Emit(_)
        | TaskDefinition::Listen(_)
        | TaskDefinition::Raise(_)
        | TaskDefinition::Run(_)
        | TaskDefinition::Set(_)
        | TaskDefinition::Switch(_)
        | TaskDefinition::Wait(_) => {}
    }
}
