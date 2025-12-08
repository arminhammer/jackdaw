use clap::Parser;
use console::style;
use serde_json::Value;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::expressions;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Invalid workflow file: {message}"))]
    InvalidWorkflowFile { message: String },

    #[snafu(display("Path error: {message}"))]
    Path { message: String },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("YAML parsing error: {source}"))]
    Yaml { source: serde_yaml::Error },

    #[snafu(display("Expression validation error: {message}"))]
    ExpressionValidation { message: String },

    #[snafu(display("Graph validation error: {source}"))]
    GraphValidation { source: crate::durableengine::Error },

    #[snafu(display("Validation failed with {count} error(s)"))]
    ValidationFailed { count: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { source }
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(source: serde_yaml::Error) -> Self {
        Error::Yaml { source }
    }
}

impl From<crate::durableengine::Error> for Error {
    fn from(source: crate::durableengine::Error) -> Self {
        Error::GraphValidation { source }
    }
}

#[derive(Parser, Debug)]
pub struct ValidateArgs {
    /// Workflow file(s) to validate. Can be a single file, multiple files, or a directory
    #[arg(required = true, value_name = "WORKFLOW")]
    pub workflows: Vec<PathBuf>,

    /// Show verbose output including all expressions checked
    #[arg(short = 'v', long)]
    pub verbose: bool,
}

#[derive(Debug)]
struct ValidationIssue {
    severity: IssueSeverity,
    location: String,
    message: String,
}

#[derive(Debug, PartialEq)]
enum IssueSeverity {
    Error,
    Warning,
}

pub async fn handle_validate(args: ValidateArgs) -> Result<()> {
    let workflow_files = discover_workflow_files(&args.workflows)?;

    if workflow_files.is_empty() {
        return Err(Error::InvalidWorkflowFile {
            message: "No workflow files found".to_string(),
        });
    }

    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut all_valid = true;

    for workflow_path in &workflow_files {
        println!(
            "\n{} {}",
            style("Validating:").bold().cyan(),
            workflow_path.display()
        );

        match validate_workflow(workflow_path, args.verbose).await {
            Ok((errors, warnings)) => {
                total_errors += errors;
                total_warnings += warnings;

                if errors > 0 {
                    all_valid = false;
                    println!(
                        "  {} {} error(s), {} warning(s)",
                        style("✗").red().bold(),
                        errors,
                        warnings
                    );
                } else if warnings > 0 {
                    println!("  {} {} warning(s)", style("⚠").yellow().bold(), warnings);
                } else {
                    println!("  {} Valid", style("✓").green().bold());
                }
            }
            Err(e) => {
                all_valid = false;
                total_errors += 1;
                println!("  {} {}", style("✗").red().bold(), e);
            }
        }
    }

    println!("\n{}", style("═".repeat(60)).dim());
    println!(
        "{} {} workflow(s) validated",
        style("Summary:").bold(),
        workflow_files.len()
    );
    println!(
        "  {} error(s), {} warning(s)",
        if total_errors > 0 {
            style(total_errors.to_string()).red().bold()
        } else {
            style(total_errors.to_string()).green()
        },
        if total_warnings > 0 {
            style(total_warnings.to_string()).yellow()
        } else {
            style(total_warnings.to_string()).dim()
        }
    );

    if !all_valid {
        return Err(Error::ValidationFailed {
            count: total_errors,
        });
    }

    Ok(())
}

async fn validate_workflow(workflow_path: &PathBuf, verbose: bool) -> Result<(usize, usize)> {
    let mut issues: Vec<ValidationIssue> = Vec::new();

    // 1. Parse the workflow
    let workflow_yaml = std::fs::read_to_string(workflow_path)?;
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)?;

    // 2. Validate graph structure
    if verbose {
        println!("  {} Validating graph structure...", style("→").dim());
    }
    match crate::durableengine::DurableEngine::validate_workflow_graph(&workflow) {
        Ok((graph, _task_names)) => {
            if verbose {
                println!(
                    "    {} Graph has {} nodes",
                    style("✓").green(),
                    graph.node_count()
                );
            }
        }
        Err(e) => {
            issues.push(ValidationIssue {
                severity: IssueSeverity::Error,
                location: "graph".to_string(),
                message: format!("Graph structure error: {}", e),
            });
        }
    }

    // 3. Extract and validate all expressions
    if verbose {
        println!("  {} Validating expressions...", style("→").dim());
    }
    let expressions = extract_all_expressions(&workflow);

    if verbose && !expressions.is_empty() {
        println!("    Found {} expression(s) to validate", expressions.len());
    }

    for (location, expr) in expressions {
        if let Err(e) = validate_expression_syntax(&expr) {
            issues.push(ValidationIssue {
                severity: IssueSeverity::Error,
                location: location.clone(),
                message: format!("Expression '{}': {}", expr, e),
            });
        } else if verbose {
            println!("    {} {}: {}", style("✓").green(), location, expr);
        }
    }

    // 4. Validate references
    if verbose {
        println!("  {} Validating references...", style("→").dim());
    }
    validate_references(&workflow, &mut issues);

    // 5. Report issues
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Error)
        .collect();
    let warnings: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Warning)
        .collect();

    for issue in &errors {
        println!(
            "  {} [{}] {}",
            style("ERROR").red().bold(),
            style(&issue.location).yellow(),
            issue.message
        );
    }

    for issue in &warnings {
        println!(
            "  {} [{}] {}",
            style("WARN").yellow().bold(),
            style(&issue.location).yellow(),
            issue.message
        );
    }

    Ok((errors.len(), warnings.len()))
}

fn discover_workflow_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut workflow_files = Vec::new();

    for path in paths {
        if !path.exists() {
            return Err(Error::Path {
                message: format!("Path does not exist: {}", path.display()),
            });
        }

        if path.is_file() {
            if is_workflow_file(path) {
                workflow_files.push(path.clone());
            } else {
                return Err(Error::InvalidWorkflowFile {
                    message: format!(
                        "File does not have .yaml or .yml extension: {}",
                        path.display()
                    ),
                });
            }
        } else if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();
                if entry_path.is_file() && is_workflow_file(&entry_path) {
                    workflow_files.push(entry_path);
                }
            }
        }
    }

    Ok(workflow_files)
}

fn is_workflow_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "yaml" || ext == "yml")
        .unwrap_or(false)
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
                    if let Some(with) = &call_task.with {
                        for (param_name, param_value) in with {
                            extract_expressions_from_value(
                                param_value,
                                &format!("task.{}.call.with.{}", task_name, param_name),
                                &mut expressions,
                            );
                        }
                    }

                    // Extract common expressions
                    if let Some(input) = &call_task.common.input
                        && let Some(from) = &input.from
                        && let Some(from_str) = from.as_str()
                    {
                        expressions.push((
                            format!("task.{}.input.from", task_name),
                            from_str.to_string(),
                        ));
                    }
                    if let Some(output) = &call_task.common.output
                        && let Some(as_expr) = &output.as_
                        && let Some(as_str) = as_expr.as_str()
                    {
                        expressions
                            .push((format!("task.{}.output.as", task_name), as_str.to_string()));
                    }
                    if let Some(if_cond) = &call_task.common.if_ {
                        expressions.push((format!("task.{}.if", task_name), if_cond.clone()));
                    }
                }
                TaskDefinition::For(for_task) => {
                    // Extract from for 'in' expression (the collection)
                    let in_expr = &for_task.for_.in_;
                    expressions.push((format!("task.{}.for.in", task_name), in_expr.clone()));

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
    context_obj.insert(
        "input".to_string(),
        serde_json::json!({}),
    );

    let context = Value::Object(context_obj);

    // Try to evaluate the expression with a dummy context
    // This will compile the expression and check for syntax errors
    match expressions::evaluate_expression(expr, &context) {
        Ok(_) => Ok(()),
        Err(e) => {
            // Extract meaningful error message
            let error_msg = match e {
                expressions::Error::JqLoad { errors } => errors,
                expressions::Error::JqCompile { errors } => {
                    // Check if this is just a variable binding issue
                    // The expression preprocessor handles variable bindings at runtime
                    if errors.contains("variable") && errors.contains("not defined") {
                        return Ok(());
                    }
                    errors
                }
                expressions::Error::JqEvaluation { message } => {
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
                expressions::Error::Evaluation { message } => message,
            };
            Err(error_msg)
        }
    }
}

fn validate_references(workflow: &WorkflowDefinition, issues: &mut Vec<ValidationIssue>) {
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
                let function_ref = &call_task.call;

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
                        issues.push(ValidationIssue {
                            severity: IssueSeverity::Warning,
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
