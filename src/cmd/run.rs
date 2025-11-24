use clap::Parser;
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use crate::durableengine::DurableEngine;
use crate::output::filter_internal_fields;
use crate::providers::cache::RedbCache;
use crate::providers::persistence::RedbPersistence;
use crate::providers::visualization::DiagramFormat;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Invalid workflow file: {message}"))]
    InvalidWorkflowFile { message: String },

    #[snafu(display("Path error: {message}"))]
    PathError { message: String },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("YAML parsing error: {source}"))]
    Yaml { source: serde_yaml::Error },

    #[snafu(display("JSON serialization error: {source}"))]
    Json { source: serde_json::Error },

    #[snafu(display("Engine error: {source}"))]
    Engine { source: crate::durableengine::Error },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: crate::cache::Error },

    #[snafu(display("Persistence error: {source}"))]
    Persistence { source: crate::persistence::Error },

    #[snafu(display("Progress display error: {source}"))]
    Progress { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<crate::durableengine::Error> for Error {
    fn from(source: crate::durableengine::Error) -> Self {
        Error::Engine { source }
    }
}

impl From<crate::cache::Error> for Error {
    fn from(source: crate::cache::Error) -> Self {
        Error::Cache { source }
    }
}

impl From<crate::persistence::Error> for Error {
    fn from(source: crate::persistence::Error) -> Self {
        Error::Persistence { source }
    }
}

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

impl From<serde_json::Error> for Error {
    fn from(source: serde_json::Error) -> Self {
        Error::Json { source }
    }
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Workflow file(s) to execute. Can be a single file, multiple files, or a directory
    #[arg(required = true, value_name = "WORKFLOW")]
    pub workflows: Vec<PathBuf>,

    /// Path to the durable persistence database
    #[arg(short = 'd', long, default_value = "workflow.db", value_name = "PATH")]
    pub durable_db: PathBuf,

    /// Path to the cache database (if different from durable db)
    #[arg(short = 'c', long, value_name = "PATH")]
    pub cache_db: Option<PathBuf>,

    /// Run workflows in parallel
    #[arg(short = 'p', long)]
    pub parallel: bool,

    /// Enable verbose output
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Skip cache hits (force re-execution)
    #[arg(long)]
    pub no_cache: bool,

    /// Generate workflow visualization after execution
    #[arg(long)]
    pub visualize: bool,

    /// Visualization tool to use (graphviz or d2)
    #[arg(long, default_value = "d2", value_name = "VIZTOOL")]
    pub viz_tool: String,

    /// Visualization output format (svg, png, pdf, ascii)
    #[arg(long, default_value = "svg", value_name = "FORMAT")]
    pub viz_format: String,

    /// Visualization output path (optional, defaults to stdout for ascii)
    #[arg(long, value_name = "PATH")]
    pub viz_output: Option<PathBuf>,
}

/// Discover all workflow files from the provided paths
fn discover_workflow_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut workflow_files = Vec::new();

    for path in paths {
        if path.is_file() {
            // Single file
            if is_workflow_file(path) {
                workflow_files.push(path.clone());
            } else {
                return Err(Error::InvalidWorkflowFile {
                    message: format!("File {:?} is not a valid workflow file (.yaml or .yml)", path),
                });
            }
        } else if path.is_dir() {
            // Directory - recursively find all workflow files
            let entries = std::fs::read_dir(path)
                .map_err(|e| Error::Io { source: e })?;

            for entry in entries {
                let entry = entry?;
                let entry_path = entry.path();
                if entry_path.is_file() && is_workflow_file(&entry_path) {
                    workflow_files.push(entry_path);
                }
            }
        } else {
            return Err(Error::PathError {
                message: format!("Path {:?} does not exist", path),
            });
        }
    }

    if workflow_files.is_empty() {
        return Err(Error::PathError {
            message: "No workflow files found in the provided paths".to_string(),
        });
    }

    Ok(workflow_files)
}

/// Check if a file is a workflow file based on extension
fn is_workflow_file(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "yaml" || ext == "yml")
        .unwrap_or(false)
}

/// Execute a single workflow with progress indication
async fn execute_workflow(
    workflow_path: &PathBuf,
    engine: Arc<DurableEngine>,
    progress: Option<&ProgressBar>,
    verbose: bool,
) -> Result<(String, serde_json::Value, WorkflowDefinition)> {
    if let Some(pb) = progress {
        pb.set_message(format!("Loading {}", workflow_path.display()));
    }

    // Read and parse workflow
    let workflow_yaml = std::fs::read_to_string(workflow_path)?;

    let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)?;

    if let Some(pb) = progress {
        pb.set_message(format!("Executing {}", workflow.document.name));
    }

    // Execute workflow
    let instance_id = engine.start(workflow.clone()).await?;

    if verbose {
        println!(
            "{} Workflow '{}' started with instance ID: {}",
            style("✓").green(),
            workflow.document.name,
            instance_id
        );
    }

    // Get final result
    let result = engine.resume(workflow.clone(), instance_id.clone()).await?;

    if let Some(pb) = progress {
        pb.finish_with_message(format!("Completed {}", workflow_path.display()));
    }

    Ok((instance_id, result, workflow))
}

/// Parse diagram format from string
fn parse_diagram_format(format_str: &str) -> Result<DiagramFormat> {
    match format_str.to_lowercase().as_str() {
        "svg" => Ok(DiagramFormat::SVG),
        "png" => Ok(DiagramFormat::PNG),
        "pdf" => Ok(DiagramFormat::PDF),
        "ascii" => Ok(DiagramFormat::ASCII),
        _ => Err(Error::InvalidWorkflowFile {
            message: format!("Invalid format '{}'. Valid formats: svg, png, pdf, ascii", format_str),
        }),
    }
}

/// Handle the run subcommand
pub async fn handle_run(args: RunArgs, multi_progress: MultiProgress) -> Result<()> {
    // Print banner
    println!(
        "{}\n",
        style("Serverless Workflow Runtime Engine v1.0")
            .bold()
            .cyan()
    );

    // Discover workflow files
    let workflow_files = discover_workflow_files(&args.workflows)?;

    if args.verbose {
        println!(
            "{} Found {} workflow file(s):",
            style("→").cyan(),
            workflow_files.len()
        );
        for file in &workflow_files {
            println!("  • {}", file.display());
        }
        println!();
    }

    // Initialize persistence and cache
    if args.verbose {
        println!("{} Initializing databases...", style("→").cyan());
        println!("  • Durable DB: {}", args.durable_db.display());
        if let Some(ref cache_db) = args.cache_db {
            println!("  • Cache DB: {}", cache_db.display());
        } else {
            println!("  • Cache DB: {} (shared)", args.durable_db.display());
        }
        println!();
    }

    let persistence = Arc::new(RedbPersistence::new(
        args.durable_db.to_str().unwrap_or("workflow.db"),
    )?);

    let cache = if let Some(cache_db_path) = args.cache_db {
        // Separate cache database
        let cache_persistence = Arc::new(RedbPersistence::new(
            cache_db_path.to_str().unwrap_or("cache.db"),
        )?);
        Arc::new(RedbCache::new(cache_persistence.db.clone())?)
    } else {
        // Shared database
        Arc::new(RedbCache::new(persistence.db.clone())?)
    };

    let engine = Arc::new(DurableEngine::new(persistence.clone(), cache.clone())?);

    // Execute workflows
    if args.parallel && workflow_files.len() > 1 {
        // Parallel execution using futures::join_all
        multi_progress.println(format!(
            "{} Executing {} workflows in parallel...\n",
            style("→").cyan(),
            workflow_files.len()
        ))?;

        let futures: Vec<_> = workflow_files
            .iter()
            .map(|workflow_path| {
                let engine_clone = engine.clone();
                let verbose = args.verbose;
                let path = workflow_path.clone();
                let pb = multi_progress.add(ProgressBar::new_spinner());
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.cyan} {msg}")
                        .unwrap(),
                );
                pb.enable_steady_tick(std::time::Duration::from_millis(100));

                async move {
                    let result = execute_workflow(&path, engine_clone, Some(&pb), verbose).await;
                    pb.finish_and_clear();
                    (path, result)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        // Print results
        multi_progress.println(format!("\n{}", style("Results:").bold().green()))?;
        for (path, result) in results {
            match result {
                Ok((instance_id, output, workflow)) => {
                    multi_progress.println(format!(
                        "\n{} {}",
                        style("✓").green(),
                        style(path.display()).bold()
                    ))?;
                    if args.verbose {
                        let filtered = filter_internal_fields(&output);
                        multi_progress.println(serde_json::to_string_pretty(&filtered)?)?;
                    }

                    // Visualization if requested
                    if args.visualize {
                        let format = parse_diagram_format(&args.viz_format)?;
                        let output_path = args.viz_output.as_deref();

                        multi_progress.println(format!(
                            "\n{} Generating visualization...",
                            style("→").cyan()
                        ))?;

                        engine
                            .visualize_execution(
                                &workflow,
                                &instance_id,
                                output_path,
                                format,
                                &args.viz_tool,
                            )
                            .await?;

                        if let Some(output_path) = output_path {
                            multi_progress.println(format!(
                                "{} Visualization saved to: {}",
                                style("✓").green(),
                                output_path.display()
                            ))?;
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("{}", e);
                    multi_progress.println(format!(
                        "\n{} {} - {}",
                        style("✗").red(),
                        style(path.display()).bold(),
                        style(&error_msg).red()
                    ))?;
                    return Err(e);
                }
            }
        }
    } else {
        // Sequential execution with progress
        multi_progress.println(format!(
            "{} Executing {} workflow(s)...\n",
            style("→").cyan(),
            workflow_files.len()
        ))?;

        let pb = multi_progress.add(ProgressBar::new(workflow_files.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        for workflow_path in workflow_files {
            match execute_workflow(&workflow_path, engine.clone(), Some(&pb), args.verbose).await {
                Ok((instance_id, result, workflow)) => {
                    multi_progress.println(format!(
                        "{} Completed: {}",
                        style("✓").green(),
                        style(workflow_path.display()).bold()
                    ))?;
                    if args.verbose {
                        let filtered = filter_internal_fields(&result);
                        multi_progress.println(serde_json::to_string_pretty(&filtered)?)?;
                    }

                    // Visualization if requested
                    if args.visualize {
                        let format = parse_diagram_format(&args.viz_format)?;
                        let output_path = args.viz_output.as_deref();

                        multi_progress.println(format!(
                            "\n{} Generating visualization...",
                            style("→").cyan()
                        ))?;

                        engine
                            .visualize_execution(
                                &workflow,
                                &instance_id,
                                output_path,
                                format,
                                &args.viz_tool,
                            )
                            .await?;

                        if let Some(output_path) = output_path {
                            multi_progress.println(format!(
                                "{} Visualization saved to: {}",
                                style("✓").green(),
                                output_path.display()
                            ))?;
                        }
                    }
                }
                Err(e) => {
                    multi_progress.println(format!(
                        "{} Failed: {} - {}",
                        style("✗").red(),
                        style(workflow_path.display()).bold(),
                        style(&e).red()
                    ))?;
                    return Err(e);
                }
            }
            pb.inc(1);
        }

        pb.finish_with_message("All workflows completed");
    }

    println!(
        "\n{} {}",
        style("✓").green().bold(),
        style("All workflows executed successfully").bold()
    );

    Ok(())
}
