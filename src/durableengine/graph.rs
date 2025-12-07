use petgraph::{graph::DiGraph, stable_graph::NodeIndex};
use serverless_workflow_core::models::task::TaskDefinition;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::collections::HashMap;

use super::{Error, Result};

/// Build an execution graph from a workflow definition
///
/// Returns a tuple of (graph, ``task_name_to_node_index_map``)
pub(super) fn build_graph(
    workflow: &WorkflowDefinition,
) -> Result<(
    DiGraph<(String, TaskDefinition), ()>,
    HashMap<String, NodeIndex>,
)> {
    let mut graph = DiGraph::new();
    let mut nodes = HashMap::new();
    let mut task_names = Vec::new();

    // Iterate over all task entries in the Map and preserve order
    for entry in &workflow.do_.entries {
        for (name, task) in entry {
            let node = graph.add_node((name.clone(), task.clone()));
            nodes.insert(name.clone(), node);
            task_names.push(name.clone());
        }
    }

    // Build explicit edges based on 'then' transitions
    let mut has_explicit_transitions = false;
    for entry in &workflow.do_.entries {
        for (name, task) in entry {
            let src = nodes.get(name).ok_or(Error::TaskExecution {
                message: "Task not found".to_string(),
            })?;
            let transitions = get_task_transitions(task);
            if !transitions.is_empty() {
                has_explicit_transitions = true;
                for target in transitions {
                    if let Some(&dst) = nodes.get(&target) {
                        graph.add_edge(*src, dst, ());
                    }
                }
            }
        }
    }

    // If no explicit transitions, create implicit sequential edges
    if !has_explicit_transitions && task_names.len() > 1 {
        for i in 0..task_names.len() - 1 {
            let src_name = task_names.get(i).ok_or(Error::TaskExecution {
                message: "Task not found".to_string(),
            })?;
            let dst_name = task_names.get(i + 1).ok_or(Error::TaskExecution {
                message: "Task not found".to_string(),
            })?;
            let src = nodes.get(src_name).ok_or(Error::TaskExecution {
                message: "Task not found".to_string(),
            })?;
            let dst = nodes.get(dst_name).ok_or(Error::TaskExecution {
                message: "Task not found".to_string(),
            })?;
            graph.add_edge(*src, *dst, ());
        }
    }

    Ok((graph, nodes))
}

/// Extract all transition targets from a task definition
///
/// Returns a vector of task names that this task can transition to
pub(super) fn get_task_transitions(task: &TaskDefinition) -> Vec<String> {
    match task {
        TaskDefinition::Call(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Set(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Fork(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Switch(t) => {
            let mut transitions = Vec::new();
            for entry in &t.switch.entries {
                for case in entry.values() {
                    if let Some(then) = &case.then {
                        transitions.push(then.clone());
                    }
                }
            }
            transitions
        }
        TaskDefinition::Do(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::Emit(t) => t
            .common
            .then
            .as_ref()
            .map(|s| vec![s.clone()])
            .unwrap_or_default(),
        TaskDefinition::For(_)
        | TaskDefinition::Listen(_)
        | TaskDefinition::Raise(_)
        | TaskDefinition::Run(_)
        | TaskDefinition::Try(_)
        | TaskDefinition::Wait(_) => vec![],
    }
}
