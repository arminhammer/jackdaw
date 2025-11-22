//! Pretty output formatting for workflow execution
//!
//! This module provides structured, colorized output for all workflow execution events.

use console::style;
use serde_json::Value;

/// Format a workflow start message
pub fn format_workflow_start(workflow_name: &str, instance_id: &str) {
    println!("\n{}", "═".repeat(80));
    println!(
        "{} {} {}",
        style("▶").cyan().bold(),
        style("Workflow:").bold(),
        style(workflow_name).cyan().bold()
    );
    println!(
        "  {} {}",
        style("Instance ID:").dim(),
        style(instance_id).dim()
    );
    println!("{}", "─".repeat(80));
}

/// Format workflow resumption message
pub fn format_workflow_resume(instance_id: &str, from_task: Option<&str>) {
    println!("\n{}", "═".repeat(80));
    println!(
        "{} {}",
        style("↻").yellow().bold(),
        style("Resuming Workflow").bold()
    );
    println!(
        "  {} {}",
        style("Instance ID:").dim(),
        style(instance_id).dim()
    );
    if let Some(task) = from_task {
        println!("  {} {}", style("From task:").dim(), style(task).yellow());
    }
    println!("{}", "─".repeat(80));
}

/// Format workflow context
pub fn format_context(title: &str, context: &Value) {
    println!("\n{}", style(title).bold());
    println!("{}", "┄".repeat(80));
    if let Some(obj) = context.as_object() {
        if obj.is_empty() {
            println!("  {}", style("(empty)").dim());
        } else {
            for (key, value) in obj.iter() {
                println!("  {}", style(format!("{}:", key)));
                println!("{}", indent_json(value, 4));
            }
        }
    } else {
        println!("{}", indent_json(context, 2));
    }
}

/// Format workflow input
pub fn format_workflow_input(input: &Value) {
    format_context("Workflow Input", input);
}

/// Format workflow output
pub fn format_workflow_output(output: &Value) {
    println!("\n{}", "═".repeat(80));
    println!("{}", style("Workflow Completed").green().bold());
    println!("{}", "─".repeat(80));
    println!("{}", style("Output").bold());
    println!("{}", "┄".repeat(80));
    println!("{}", indent_json(output, 2));
    println!("{}", "═".repeat(80));
}

/// Format task execution start
pub fn format_task_start(task_name: &str, task_type: &str) {
    println!("\n{}", "─".repeat(80));
    println!(
        "{} {} {} {}",
        style("▸").cyan(),
        style("Task:").bold(),
        style(task_name).cyan(),
        style(format!("[{}]", task_type)).dim()
    );
    println!("{}", "┄".repeat(80));
}

/// Format task already completed (from checkpoint)
pub fn format_task_skipped(task_name: &str) {
    println!(
        "  {} {}",
        style("⤼").yellow(),
        style(format!("Skipping '{}' (already completed)", task_name)).yellow()
    );
}

/// Format cache hit
pub fn format_cache_hit(_task_name: &str, key: &str, timestamp: Option<&str>) {
    println!("  {}", style("Cache Hit").yellow().bold());
    println!("  {}", "·".repeat(78));
    println!("    {} {}", style("Key:").yellow(), style(key).yellow());
    if let Some(ts) = timestamp {
        println!(
            "    {} {}",
            style("Cached at:").yellow(),
            style(ts).yellow()
        );
    }
}

/// Format cache miss
pub fn format_cache_miss(_task_name: &str, key: &str) {
    println!("  {}", style("Cache Miss").yellow());
    println!("  {}", "·".repeat(78));
    println!("    {} {}", style("Key:").yellow(), style(key).yellow());
}

/// Filter out internal descriptor fields from display
fn filter_internal_fields(value: &Value) -> Value {
    if let Some(obj) = value.as_object() {
        let filtered: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(key, _)| !key.starts_with("__"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Value::Object(filtered)
    } else {
        value.clone()
    }
}

/// Format task context
pub fn format_task_context(context: &Value) {
    println!("  {}", style("Context").white());
    println!("  {}", "·".repeat(78));
    let filtered = filter_internal_fields(context);
    if let Some(obj) = filtered.as_object() {
        if obj.is_empty() {
            println!("    {}", style("(empty)").dim());
        } else {
            for (key, value) in obj.iter() {
                println!("    {}", style(format!("{}:", key)).white());
                println!("{}", colorize_json(value, 6, "white"));
            }
        }
    } else {
        println!("{}", colorize_json(&filtered, 4, "white"));
    }
}

/// Format task input
pub fn format_task_input(input: &Value) {
    println!("  {}", style("Input").cyan());
    println!("  {}", "·".repeat(78));
    let filtered = filter_internal_fields(input);
    if let Some(obj) = filtered.as_object() {
        if obj.is_empty() {
            println!("    {}", style("(empty)").dim());
        } else {
            for (key, value) in obj.iter() {
                println!("    {}", style(format!("{}:", key)).cyan());
                println!("{}", colorize_json(value, 6, "cyan"));
            }
        }
    } else {
        println!("{}", colorize_json(&filtered, 4, "cyan"));
    }
}

/// Format task output
pub fn format_task_output(output: &Value) {
    println!("  {}", style("Output").green());
    println!("  {}", "·".repeat(78));
    if let Some(obj) = output.as_object() {
        if obj.is_empty() {
            println!("    {}", style("(empty)").dim());
        } else {
            for (key, value) in obj.iter() {
                println!("    {}", style(format!("{}:", key)).green());
                println!("{}", colorize_json(value, 6, "green"));
            }
        }
    } else {
        println!("{}", colorize_json(output, 4, "green"));
    }
}

/// Format task stdout/stderr
pub fn format_task_logs(stdout: Option<&str>, stderr: Option<&str>) {
    if let Some(out) = stdout {
        if !out.trim().is_empty() {
            println!("  {}", style("Stdout").dim());
            for line in out.lines() {
                println!("    {}", style(line).dim());
            }
        }
    }

    if let Some(err) = stderr {
        if !err.trim().is_empty() {
            println!("  {}", style("Stderr").yellow());
            for line in err.lines() {
                println!("    {}", style(line).yellow());
            }
        }
    }
}

/// Format task completion
pub fn format_task_complete(task_name: &str) {
    println!(
        "  {} {}",
        style("✓").green(),
        style(format!("Completed '{}'", task_name)).green()
    );
}

/// Format task error
pub fn format_task_error(task_name: &str, error: &str) {
    println!(
        "  {} {}",
        style("✗").red().bold(),
        style(format!("Failed '{}'", task_name)).red().bold()
    );
    println!("    {} {}", style("Error:").red(), style(error).red());
}

/// Format fork branch execution
pub fn format_fork_start(fork_name: &str, branch_count: usize) {
    println!(
        "\n{} {} {} {}",
        style("⋔").cyan(),
        style("Fork:").bold(),
        style(fork_name).cyan(),
        style(format!("[{} branches]", branch_count)).dim()
    );
}

/// Format branch execution
pub fn format_branch_start(branch_name: &str, task_type: &str) {
    println!(
        "  {} {} {}",
        style("├─").dim(),
        style(branch_name).cyan(),
        style(format!("[{}]", task_type)).dim()
    );
}

/// Helper: Format a value with full content (no truncation)
fn format_value_full(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// Helper: Indent JSON output
fn indent_json(value: &Value, indent: usize) -> String {
    let json_str = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    let indent_str = " ".repeat(indent);
    json_str
        .lines()
        .map(|line| format!("{}{}", indent_str, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Helper: Colorize JSON output
fn colorize_json(value: &Value, indent: usize, color: &str) -> String {
    let json_str = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    let indent_str = " ".repeat(indent);
    let styled_lines: Vec<String> = json_str
        .lines()
        .map(|line| {
            let styled = match color {
                "white" => style(line).white(),
                "cyan" => style(line).cyan(),
                "blue" => style(line).blue(),
                "green" => style(line).green(),
                "red" => style(line).red(),
                "yellow" => style(line).yellow(),
                _ => style(line),
            };
            format!("{}{}", indent_str, styled)
        })
        .collect();
    styled_lines.join("\n")
}
