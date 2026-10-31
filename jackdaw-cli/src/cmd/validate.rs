use clap::Parser;
use console::style;
use jackdaw::api::{Severity, Workflow};
use jackdaw::dsl::workflow::WorkflowDefinition;
use serde_json::Value;
use snafu::prelude::*;
use std::path::{Path, PathBuf};

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

#[derive(Parser, Debug)]
pub struct ValidateArgs {
    /// Workflow file(s) to validate. Can be a single file, multiple files, or a directory
    #[arg(required = true, value_name = "WORKFLOW")]
    pub workflows: Vec<PathBuf>,

    /// Show verbose output including all expressions checked
    #[arg(short = 'v', long)]
    pub verbose: bool,
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
    let workflow_yaml = std::fs::read_to_string(workflow_path)?;
    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)?;
    let validation = Workflow::<Value, Value>::from_definition(workflow).validate();

    if verbose {
        println!("  {} Validating graph structure...", style("→").dim());
        if validation.graph_nodes > 0 {
            println!(
                "    {} Graph has {} nodes",
                style("✓").green(),
                validation.graph_nodes
            );
        }
        println!("  {} Validating expressions...", style("→").dim());
        if !validation.expressions.is_empty() {
            println!(
                "    Found {} expression(s) to validate",
                validation.expressions.len()
            );
        }
        for (location, expr) in &validation.expressions {
            let failed = validation
                .issues
                .iter()
                .any(|issue| issue.location == *location);
            if !failed {
                println!("    {} {}: {}", style("✓").green(), location, expr);
            }
        }
        println!("  {} Validating references...", style("→").dim());
    }

    for issue in validation.of(Severity::Error) {
        println!(
            "  {} [{}] {}",
            style("ERROR").red().bold(),
            style(&issue.location).yellow(),
            issue.message
        );
    }
    for issue in validation.of(Severity::Warning) {
        println!(
            "  {} [{}] {}",
            style("WARN").yellow().bold(),
            style(&issue.location).yellow(),
            issue.message
        );
    }

    Ok((
        validation.of(Severity::Error).count(),
        validation.of(Severity::Warning).count(),
    ))
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
