//! Which names a workflow definition calls, at any depth. Used to reject a workflow that calls something nobody
//! provides before any of its tasks run.
use crate::task_ext::call_name;
use serverless_workflow_core::models::map::Map;
use serverless_workflow_core::models::task::{TaskDefinition, TaskDefinitionFields};
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

/// The `then` directives that name no task: each must be `continue`, `exit`, `end`, or a task in the same list.
/// Returned as `task -> target`.
pub(crate) fn unknown_directives(definition: &WorkflowDefinition) -> Vec<String> {
    let mut unknown = Vec::new();
    check_map(&definition.do_, &mut unknown);
    unknown
}

fn check_map(tasks: &Map<String, TaskDefinition>, unknown: &mut Vec<String>) {
    let siblings: Vec<&String> = tasks
        .entries
        .iter()
        .flat_map(|entry| entry.keys())
        .collect();
    let mut check = |task: &str, then: Option<&String>| {
        if let Some(then) = then
            && !matches!(then.as_str(), "continue" | "exit" | "end")
            && !siblings.contains(&then)
        {
            unknown.push(format!("{task} -> {then}"));
        }
    };
    for entry in &tasks.entries {
        for (name, task) in entry {
            check(name, common(task).then.as_ref());
            if let TaskDefinition::Switch(switch) = task {
                for case in switch
                    .switch
                    .entries
                    .iter()
                    .flat_map(|entry| entry.values())
                {
                    check(name, case.then.as_ref());
                }
            }
        }
    }
    for task in tasks.entries.iter().flat_map(|entry| entry.values()) {
        match task {
            TaskDefinition::Do(do_task) => check_map(&do_task.do_, unknown),
            TaskDefinition::For(for_task) => check_map(&for_task.do_, unknown),
            TaskDefinition::Fork(fork_task) => check_map(&fork_task.fork.branches, unknown),
            TaskDefinition::Try(try_task) => {
                check_map(&try_task.try_, unknown);
                if let Some(handler) = &try_task.catch.do_ {
                    check_map(handler, unknown);
                }
            }
            TaskDefinition::Call(_)
            | TaskDefinition::Emit(_)
            | TaskDefinition::Listen(_)
            | TaskDefinition::Raise(_)
            | TaskDefinition::Run(_)
            | TaskDefinition::Set(_)
            | TaskDefinition::Switch(_)
            | TaskDefinition::Wait(_) => {}
        }
    }
}

fn common(task: &TaskDefinition) -> &TaskDefinitionFields {
    match task {
        TaskDefinition::Call(t) => t.common(),
        TaskDefinition::Do(t) => &t.common,
        TaskDefinition::Emit(t) => &t.common,
        TaskDefinition::For(t) => &t.common,
        TaskDefinition::Fork(t) => &t.common,
        TaskDefinition::Listen(t) => &t.common,
        TaskDefinition::Raise(t) => &t.common,
        TaskDefinition::Run(t) => &t.common,
        TaskDefinition::Set(t) => &t.common,
        TaskDefinition::Switch(t) => &t.common,
        TaskDefinition::Try(t) => &t.common,
        TaskDefinition::Wait(t) => &t.common,
    }
}
