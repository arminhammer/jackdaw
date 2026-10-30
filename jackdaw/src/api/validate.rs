//! Checking a YAML workflow without running it: [`Workflow::validate`](super::Workflow::validate).
use crate::task_ext::call_parts;
use serde_json::Value;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::collections::HashSet;

/// How serious a [`Issue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// The workflow cannot run as written.
    Error,
    /// The workflow may run, but probably not as intended.
    Warning,
}

/// One problem found by [`Workflow::validate`](super::Workflow::validate).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issue {
    /// How serious it is.
    pub severity: Severity,
    /// Where it is, e.g. `task.charge.call.with.amount` or `graph`.
    pub location: String,
    /// What is wrong.
    pub message: String,
}

/// What [`Workflow::validate`](super::Workflow::validate) checked and found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Validation {
    /// The problems, errors first.
    pub issues: Vec<Issue>,
    /// The nodes of the workflow's task graph (0 when the graph could not be built).
    pub graph_nodes: usize,
    /// The expressions that were checked, with where they are.
    pub expressions: Vec<(String, String)>,
}

impl Validation {
    /// The issues of `severity`.
    pub fn of(&self, severity: Severity) -> impl Iterator<Item = &Issue> {
        self.issues
            .iter()
            .filter(move |issue| issue.severity == severity)
    }

    /// Whether no issue is an error.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.of(Severity::Error).next().is_none()
    }
}

/// Checks the task graph, the syntax of every `${ }` expression, and that `call`s name a defined function.
pub(crate) fn validate(workflow: &WorkflowDefinition) -> Validation {
    let mut issues = Vec::new();
    let graph_nodes = match crate::durableengine::DurableEngine::validate_workflow_graph(workflow) {
        Ok((graph, _)) => graph.node_count(),
        Err(error) => {
            issues.push(Issue {
                severity: Severity::Error,
                location: "graph".to_string(),
                message: format!("Graph structure error: {error}"),
            });
            0
        }
    };
    let expressions = extract_all_expressions(workflow);
    for (location, expr) in &expressions {
        if let Err(error) = validate_expression_syntax(expr) {
            issues.push(Issue {
                severity: Severity::Error,
                location: location.clone(),
                message: format!("Expression '{expr}': {error}"),
            });
        }
    }
    validate_references(workflow, &mut issues);
    issues.sort_by_key(|issue| issue.severity != Severity::Error);
    Validation {
        issues,
        graph_nodes,
        expressions,
    }
}

fn extract_all_expressions(workflow: &WorkflowDefinition) -> Vec<(String, String)> {
    let mut expressions = Vec::new();

    // Extract from input filter
    if let Some(input) = &workflow.input
        && let Some(from) = &input.from
        && let Some(from_str) = from.as_str()
    {
        expressions.push(("input.from".to_string(), from_str.to_string()));
    }

    // Extract from output filter
    if let Some(output) = &workflow.output
        && let Some(as_expr) = &output.as_
        && let Some(as_str) = as_expr.as_str()
    {
        expressions.push(("output.as".to_string(), as_str.to_string()));
    }

    // Extract from tasks - TaskDefinition is an enum, so we need to match on it
    use serverless_workflow_core::models::task::TaskDefinition;

    for entry in &workflow.do_.entries {
        for (task_name, task_def) in entry {
            // Extract task-specific expressions based on task type
            match task_def {
                TaskDefinition::Set(set_task) => {
                    use serverless_workflow_core::models::task::SetValue;

                    // Extract from set values
                    match &set_task.set {
                        SetValue::Map(map) => {
                            for (var_name, value) in map {
                                if let Some(val_str) = value.as_str()
                                    && val_str.starts_with("${")
                                    && val_str.ends_with("}")
                                {
                                    expressions.push((
                                        format!("task.{}.set.{}", task_name, var_name),
                                        val_str.to_string(),
                                    ));
                                }
                            }
                        }
                        SetValue::Expression(expr) => {
                            // The entire set value is an expression
                            expressions.push((format!("task.{}.set", task_name), expr.clone()));
                        }
                    }

                    // Extract common expressions
                    if let Some(input) = &set_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &set_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &set_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Call(call_task) => {
                    // Extract from call 'with' parameters
                    if let Ok((_, with)) = call_parts(call_task) {
                        for (param_name, param_value) in &with {
                            extract_expressions_from_value(
                                param_value,
                                &format!("task.{}.call.with.{}", task_name, param_name),
                                &mut expressions,
                            );
                        }
                    }

                    // Extract common expressions
                    if let Some(input) = &call_task.common().input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &call_task.common().output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &call_task.common().if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::For(for_task) => {
                    // Extract from for 'in' expression (the collection)
                    if let Some(in_expr) = for_task.for_.in_.as_str() {
                        expressions
                            .push((format!("task.{}.for.in", task_name), in_expr.to_string()));
                    }

                    // Extract common expressions
                    if let Some(input) = &for_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &for_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &for_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Switch(switch_task) => {
                    // Extract from switch 'when' conditions
                    for (idx, entry) in switch_task.switch.entries.iter().enumerate() {
                        for case_def in entry.values() {
                            if let Some(when_expr) = &case_def.when {
                                expressions.push((
                                    format!("task.{}.switch[{}].when", task_name, idx),
                                    when_expr.clone(),
                                ));
                            }
                        }
                    }

                    // Extract common expressions
                    if let Some(input) = &switch_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &switch_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &switch_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Run(run_task) => {
                    // Extract common expressions
                    if let Some(input) = &run_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &run_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &run_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Do(do_task) => {
                    // Extract common expressions
                    if let Some(input) = &do_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &do_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &do_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Fork(fork_task) => {
                    // Extract common expressions
                    if let Some(input) = &fork_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &fork_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &fork_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Try(try_task) => {
                    // Extract common expressions
                    if let Some(input) = &try_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &try_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &try_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Emit(emit_task) => {
                    // Extract common expressions
                    if let Some(input) = &emit_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &emit_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &emit_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Raise(raise_task) => {
                    // Extract common expressions
                    if let Some(input) = &raise_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &raise_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &raise_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Wait(wait_task) => {
                    // Extract common expressions
                    if let Some(input) = &wait_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &wait_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &wait_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::Listen(listen_task) => {
                    // Extract common expressions
                    if let Some(input) = &listen_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &listen_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &listen_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
            }
        }
    }

    expressions
}

fn extract_expressions_from_value(
    value: &Value,
    location: &str,
    expressions: &mut Vec<(String, String)>,
) {
    match value {
        Value::String(s) => {
            if s.starts_with("${") && s.ends_with('}') {
                expressions.push((location.to_string(), s.clone()));
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                extract_expressions_from_value(val, &format!("{location}.{key}"), expressions);
            }
        }
        Value::Array(arr) => {
            for (idx, val) in arr.iter().enumerate() {
                extract_expressions_from_value(val, &format!("{location}[{idx}]"), expressions);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn validate_expression_syntax(expr: &str) -> std::result::Result<(), String> {
    // Create a dummy context with special variables that Jackdaw supports
    // This allows expressions like $workflow.id, $input.data, etc. to compile
    let mut context_obj = serde_json::Map::new();

    // Add dummy __workflow descriptor
    context_obj.insert(
        "__workflow".to_string(),
        serde_json::json!({
            "id": "dummy-workflow-id",
            "name": "dummy-workflow",
            "version": "1.0.0"
        }),
    );

    // Add dummy __runtime descriptor
    context_obj.insert(
        "__runtime".to_string(),
        serde_json::json!({
            "version": "1.0.0"
        }),
    );

    // Add dummy input for $input references
    context_obj.insert("input".to_string(), serde_json::json!({}));

    let context = Value::Object(context_obj);

    // Try to evaluate the expression with a dummy context
    // This will compile the expression and check for syntax errors
    match crate::expressions::evaluate_expression(expr, &context) {
        Ok(_) => Ok(()),
        Err(e) => {
            // Extract meaningful error message
            let error_msg = match e {
                crate::expressions::Error::JqLoad { errors } => errors,
                crate::expressions::Error::JqCompile { errors } => {
                    // Check if this is just a variable binding issue
                    // The expression preprocessor handles variable bindings at runtime
                    if errors.contains("variable") && errors.contains("not defined") {
                        return Ok(());
                    }
                    errors
                }
                crate::expressions::Error::JqEvaluation { message } => {
                    // For validation, we don't care about runtime errors like "null has no field"
                    // These are expected since we're using a dummy context
                    if message.contains("has no")
                        || message.contains("cannot")
                        || message.contains("not defined")
                    {
                        return Ok(());
                    }
                    message
                }
                crate::expressions::Error::Evaluation { message } => message,
            };
            Err(error_msg)
        }
    }
}

fn validate_references(workflow: &WorkflowDefinition, issues: &mut Vec<Issue>) {
    use serverless_workflow_core::models::task::TaskDefinition;

    // Collect defined functions
    let mut defined_functions = HashSet::new();
    let mut has_catalogs = false;

    if let Some(use_) = &workflow.use_ {
        // Collect inline function definitions
        if let Some(functions) = &use_.functions {
            for func_name in functions.keys() {
                defined_functions.insert(func_name.clone());
            }
        }

        // Check if catalogs are defined
        if let Some(catalogs) = &use_.catalogs {
            has_catalogs = !catalogs.is_empty();
        }
    }

    // Check function references in 'call' tasks
    for entry in &workflow.do_.entries {
        for (task_name, task_def) in entry {
            if let TaskDefinition::Call(call_task) = task_def {
                let Ok((function_ref, _)) = call_parts(call_task) else {
                    continue;
                };

                // Skip HTTP/HTTPS calls and catalog references
                if function_ref.starts_with("http://")
                    || function_ref.starts_with("https://")
                    || function_ref.contains('#')
                {
                    continue;
                }

                // If function is not defined inline, check if it might come from a catalog
                if !defined_functions.contains(function_ref) {
                    // If catalogs are defined, the function might be resolved from there
                    // We can't validate catalog contents at validation time, so only warn
                    // if there are no catalogs defined at all
                    if !has_catalogs {
                        issues.push(Issue {
                            severity: Severity::Warning,
                            location: format!("task.{task_name}.call"),
                            message: format!(
                                "Function '{function_ref}' is not defined in 'use.functions' and no catalogs are configured"
                            ),
                        });
                    }
                }
            }
        }
    }
}
