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
            for (key, value) in obj {
                println!("  {}", style(format!("{key}:")));
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

    let filtered = filter_internal_fields(output);

    // Only show output section if there's meaningful data to display
    // Skip if it's just stdout/stderr/exitCode (which was already streamed)
    if let Some(obj) = filtered.as_object() {
        let is_just_script_output = obj.len() <= 3
            && obj.contains_key("stdout")
            && obj.contains_key("stderr")
            && obj.contains_key("exitCode");

        if !is_just_script_output && !obj.is_empty() {
            println!("{}", style("Output").bold());
            println!("{}", "┄".repeat(80));
            println!("{}", indent_json(&filtered, 2));
        }
    } else if !filtered.is_null() {
        println!("{}", style("Output").bold());
        println!("{}", "┄".repeat(80));
        println!("{}", indent_json(&filtered, 2));
    }

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
        style(format!("[{task_type}]")).dim()
    );
    println!("{}", "┄".repeat(80));
}

/// Format task already completed (from checkpoint)
pub fn format_task_skipped(task_name: &str) {
    println!(
        "  {} {}",
        style("⤼").yellow(),
        style(format!("Skipping '{task_name}' (already completed)")).yellow()
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
#[must_use]
pub fn filter_internal_fields(value: &Value) -> Value {
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
            for (key, value) in obj {
                println!("    {}", style(format!("{key}:")).white());
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
            for (key, value) in obj {
                println!("    {}", style(format!("{key}:")).cyan());
                println!("{}", colorize_json(value, 6, "cyan"));
            }
        }
    } else {
        println!("{}", colorize_json(&filtered, 4, "cyan"));
    }
}

/// Format run task parameters (stdin, arguments, environment)
pub fn format_run_task_params(
    language: Option<&str>,
    stdin: Option<&str>,
    arguments: Option<&Value>,
    environment: Option<&Value>,
) {
    if let Some(lang) = language {
        println!("  {} {}", style("Language:").cyan(), style(lang).cyan());
    }

    if let Some(stdin_val) = stdin {
        println!(
            "  {} {}",
            style("Stdin:").cyan(),
            style(format!("\"{stdin_val}\"")).cyan()
        );
    }

    if let Some(arr) = arguments
        .and_then(|args| args.as_array())
        .filter(|a| !a.is_empty())
    {
        println!("  {}", style("Arguments:").cyan());
        for arg in arr {
            if let Some(s) = arg.as_str() {
                println!("    {}", style(format!("- {s}")).cyan());
            }
        }
    }

    if let Some(env) = environment {
        if let Some(obj) = env.as_object() {
            if !obj.is_empty() {
                println!("  {}", style("Environment:").cyan());
                for (key, value) in obj {
                    if let Some(s) = value.as_str() {
                        println!(
                            "    {} {}",
                            style(format!("{}:", key)).cyan(),
                            style(s).cyan()
                        );
                    }
                }
            }
        }
    }
}

/// Format task output
pub fn format_task_output(output: &Value) {
    println!("  {}", style("Output").green());
    println!("  {}", "·".repeat(78));

    let filtered = filter_internal_fields(output);

    // For streamed output, show stdout/stderr based on exit code
    if output
        .get("__streamed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        if let Some(obj) = filtered.as_object() {
            let exit_code = obj
                .get("exitCode")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);

            if exit_code == 0 {
                // Success: Only show stdout value (no stderr, no exitCode)
                if let Some(stdout) = obj.get("stdout").and_then(serde_json::Value::as_str) {
                    if stdout.is_empty() {
                        println!("    {}", style("(empty)").dim());
                    } else {
                        println!("    {}", style(format!("\"{stdout}\"")).green());
                    }
                } else {
                    println!("    {}", style("(empty)").dim());
                }
            } else {
                // Failure: Show both stdout and stderr for debugging (no exitCode)
                if let Some(stdout) = obj.get("stdout").and_then(serde_json::Value::as_str) {
                    if !stdout.is_empty() {
                        println!("    {}", style("stdout:").green());
                        println!("      {}", style(format!("\"{stdout}\"")).green());
                    }
                }
                if let Some(stderr) = obj.get("stderr").and_then(serde_json::Value::as_str) {
                    if !stderr.is_empty() {
                        println!("    {}", style("stderr:").green());
                        println!("      {}", style(format!("\"{stderr}\"")).green());
                    }
                }
            }
        }
        return;
    }

    // For non-streamed output, show full structure
    if let Some(obj) = filtered.as_object() {
        if obj.is_empty() {
            println!("    {}", style("(empty)").dim());
        } else {
            for (key, value) in obj {
                println!("    {}", style(format!("{key}:")).green());
                println!("{}", colorize_json(value, 6, "green"));
            }
        }
    } else {
        println!("{}", colorize_json(&filtered, 4, "green"));
    }
}

/// Format task stdout/stderr
pub fn format_task_logs(stdout: Option<&str>, stderr: Option<&str>) {
    if let Some(out) = stdout.filter(|s| !s.trim().is_empty()) {
        println!("  {}", style("Stdout").dim());
        for line in out.lines() {
            println!("    {}", style(line).dim());
        }
    }

    if let Some(err) = stderr.filter(|e| !e.trim().is_empty()) {
        println!("  {}", style("Stderr").yellow());
        for line in err.lines() {
            println!("    {}", style(line).yellow());
        }
    }
}

/// Format task completion
pub fn format_task_complete(task_name: &str) {
    println!(
        "  {} {}",
        style("✓").green(),
        style(format!("Completed '{task_name}'")).green()
    );
}

/// Format task error
pub fn format_task_error(task_name: &str, error: &str) {
    println!(
        "  {} {}",
        style("✗").red().bold(),
        style(format!("Failed '{task_name}'")).red().bold()
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
        style(format!("[{branch_count} branches]")).dim()
    );
}

/// Format branch execution
pub fn format_branch_start(branch_name: &str, task_type: &str) {
    println!(
        "  {} {} {}",
        style("├─").dim(),
        style(branch_name).cyan(),
        style(format!("[{task_type}]")).dim()
    );
}

/// Helper: Indent JSON output
fn indent_json(value: &Value, indent: usize) -> String {
    let json_str = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    let indent_str = " ".repeat(indent);
    json_str
        .lines()
        .map(|line| format!("{indent_str}{line}"))
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
            format!("{indent_str}{styled}")
        })
        .collect();
    styled_lines.join("\n")
}
