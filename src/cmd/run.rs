use clap::Parser;
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::CacheProvider;
use crate::config::JackdawConfig;
use crate::durableengine::DurableEngine;
use crate::output::filter_internal_fields;
use crate::persistence::PersistenceProvider;
use crate::providers::cache::{mem::InMemoryCache, PostgresCache, RedbCache, SqliteCache};
use crate::providers::persistence::{
    InMemoryPersistence, PostgresPersistence, RedbPersistence, SqlitePersistence,
};
use crate::providers::visualization::DiagramFormat;

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
#[allow(clippy::struct_excessive_bools)]
pub struct RunArgs {
    /// Workflow file(s) to execute. Can be a single file, multiple files, or a directory
    #[arg(required = true, value_name = "WORKFLOW")]
    pub workflows: Vec<PathBuf>,

    /// Path to the durable persistence database
    #[arg(short = 'd', long, value_name = "PATH")]
    pub durable_db: Option<PathBuf>,

    /// Path to the cache database (if different from durable db)
    #[arg(short = 'c', long, value_name = "PATH")]
    pub cache_db: Option<PathBuf>,

    /// Run workflows in parallel
    #[arg(short = 'p', long)]
    pub parallel: bool,

    /// Enable verbose output
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Enable debug mode (show detailed execution information)
    #[arg(long)]
    pub debug: bool,

    /// Persistence provider to use (memory, redb, sqlite, postgres)
    #[arg(long, value_name = "PERSISTENCE_PROVIDER", default_value = "memory")]
    pub persistence_provider: String,

    /// Cache provider to use (memory, redb, sqlite, postgres)
    #[arg(long, value_name = "CACHE_PROVIDER", default_value = "memory")]
    pub cache_provider: String,

    /// SQLite database URL (e.g., 'sqlite:workflow.db' or 'sqlite::memory:')
    #[arg(long, value_name = "SQLITE_DB_URL", env = "SQLITE_DB_URL")]
    pub sqlite_db_url: Option<String>,

    /// PostgreSQL database name
    #[arg(long, value_name = "POSTGRES_DB_NAME", env = "POSTGRES_DB_NAME")]
    pub postgres_db_name: Option<String>,

    /// PostgreSQL user
    #[arg(long, value_name = "POSTGRES_USER", env = "POSTGRES_USER")]
    pub postgres_user: Option<String>,

    /// PostgreSQL password
    #[arg(long, value_name = "POSTGRES_PASSWORD", env = "POSTGRES_PASSWORD")]
    pub postgres_password: Option<String>,

    /// PostgreSQL hostname
    #[arg(long, value_name = "POSTGRES_HOSTNAME", env = "POSTGRES_HOSTNAME")]
    pub postgres_hostname: Option<String>,

    /// Generate workflow visualization after execution
    #[arg(long)]
    pub visualize: bool,

    /// Visualization tool to use (graphviz or d2)
    #[arg(long, value_name = "VIZTOOL")]
    pub viz_tool: Option<String>,

    /// Visualization output format (svg, png, pdf, ascii)
    #[arg(long, value_name = "FORMAT")]
    pub viz_format: Option<String>,

    /// Visualization output path (optional, defaults to stdout for ascii)
    #[arg(long, value_name = "PATH")]
    pub viz_output: Option<PathBuf>,

    /// Input data for the workflow (JSON string or path to JSON file)
    #[arg(short = 'i', long, value_name = "INPUT")]
    pub input: Option<String>,

    /// Workflow registry paths - directories or files containing workflows that can be called
    #[arg(short = 'r', long = "registry", value_name = "PATH")]
    pub registry: Option<Vec<PathBuf>>,
}

impl RunArgs {
    /// Merge CLI arguments with config file settings
    /// CLI arguments take precedence over config file settings
    pub fn merge_with_config(self, config: JackdawConfig) -> JackdawConfig {
        JackdawConfig {
            durable_db: self.durable_db.or(config.durable_db),
            cache_db: self.cache_db.or(config.cache_db),
            parallel: if self.parallel { true } else { config.parallel },
            verbose: if self.verbose { true } else { config.verbose },
            visualize: if self.visualize {
                true
            } else {
                config.visualize
            },
            viz_tool: self.viz_tool.or(config.viz_tool),
            viz_format: self.viz_format.or(config.viz_format),
            viz_output: self.viz_output.or(config.viz_output),
        }
    }
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
                    message: format!(
                        "File {} is not a valid workflow file (.yaml or .yml)",
                        path.display()
                    ),
                });
            }
        } else if path.is_dir() {
            // Directory - recursively find all workflow files
            let entries = std::fs::read_dir(path).context(IoSnafu)?;

            for entry in entries {
                let entry = entry?;
                let entry_path = entry.path();
                if entry_path.is_file() && is_workflow_file(&entry_path) {
                    workflow_files.push(entry_path);
                }
            }
        } else {
            return Err(Error::Path {
                message: format!("Path {} does not exist", path.display()),
            });
        }
    }

    if workflow_files.is_empty() {
        return Err(Error::Path {
            message: "No workflow files found in the provided paths".to_string(),
        });
    }

    Ok(workflow_files)
}

/// Check if a file is a workflow file based on extension
fn is_workflow_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
}

/// Execute a single workflow with progress indication
async fn execute_workflow(
    workflow_path: &PathBuf,
    engine: Arc<DurableEngine>,
    progress: Option<&ProgressBar>,
    _verbose: bool,
    input: Option<&String>,
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

    // Parse input data
    let input_data = if let Some(input_str) = input {
        // Try to parse as JSON first
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(input_str) {
            json
        } else {
            // Try to read as file path
            let input_path = PathBuf::from(input_str);
            if input_path.exists() {
                let file_content = std::fs::read_to_string(&input_path)?;
                serde_json::from_str(&file_content)?
            } else {
                return Err(Error::InvalidWorkflowFile {
                    message: format!(
                        "Input '{}' is neither valid JSON nor a valid file path",
                        input_str
                    ),
                });
            }
        }
    } else {
        serde_json::json!({})
    };

    // Execute workflow - returns both instance_id and final result
    let (instance_id, result) = engine
        .start_with_input(workflow.clone(), input_data)
        .await?;

    if let Some(pb) = progress {
        pb.finish_with_message(format!("Completed {}", workflow_path.display()));
    }

    Ok((instance_id, result, workflow))
}

/// Parse diagram format from string
fn parse_diagram_format(format_str: &str) -> Result<DiagramFormat> {
    match format_str.to_lowercase().as_str() {
        "svg" => Ok(DiagramFormat::Svg),
        "png" => Ok(DiagramFormat::Png),
        "pdf" => Ok(DiagramFormat::Pdf),
        "ascii" => Ok(DiagramFormat::Ascii),
        _ => Err(Error::InvalidWorkflowFile {
            message: format!("Invalid format '{format_str}'. Valid formats: svg, png, pdf, ascii"),
        }),
    }
}

/// Build PostgreSQL connection URL and validate all required parameters are provided
fn build_postgres_url(
    db_name: Option<&String>,
    user: Option<&String>,
    password: Option<&String>,
    hostname: Option<&String>,
) -> Result<String> {
    let db_name = db_name.ok_or_else(|| Error::InvalidWorkflowFile {
        message: "PostgreSQL provider requires --postgres-db-name parameter or POSTGRES_DB_NAME env var".to_string(),
    })?;
    let user = user.ok_or_else(|| Error::InvalidWorkflowFile {
        message: "PostgreSQL provider requires --postgres-user parameter or POSTGRES_USER env var".to_string(),
    })?;
    let password = password.ok_or_else(|| Error::InvalidWorkflowFile {
        message: "PostgreSQL provider requires --postgres-password parameter or POSTGRES_PASSWORD env var".to_string(),
    })?;
    let hostname = hostname.ok_or_else(|| Error::InvalidWorkflowFile {
        message: "PostgreSQL provider requires --postgres-hostname parameter or POSTGRES_HOSTNAME env var".to_string(),
    })?;

    Ok(format!("postgresql://{}:{}@{}/{}", user, password, hostname, db_name))
}

/// Handle the run subcommand with graceful shutdown support
pub async fn handle_run(
    workflows: Vec<PathBuf>,
    input: Option<String>,
    registry: Option<Vec<PathBuf>>,
    config: JackdawConfig,
    multi_progress: MultiProgress,
    debug: bool,
    persistence_provider: String,
    cache_provider: String,
    sqlite_db_url: Option<String>,
    postgres_db_name: Option<String>,
    postgres_user: Option<String>,
    postgres_password: Option<String>,
    postgres_hostname: Option<String>,
) -> Result<()> {
    // Set up signal handler for graceful shutdown
    let shutdown_signal = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigint = signal(SignalKind::interrupt()).expect("Failed to create SIGINT handler");
            let mut sigterm = signal(SignalKind::terminate()).expect("Failed to create SIGTERM handler");

            tokio::select! {
                _ = sigint.recv() => {
                    eprintln!("\nReceived SIGINT (Ctrl+C), shutting down gracefully...");
                }
                _ = sigterm.recv() => {
                    eprintln!("\nReceived SIGTERM, shutting down gracefully...");
                }
            }
        }

        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for Ctrl+C");
            eprintln!("\nReceived Ctrl+C, shutting down gracefully...");
        }
    };

    // Run the workflow execution with shutdown signal handling
    tokio::select! {
        result = run_workflows_internal(
            workflows,
            input,
            registry,
            config,
            multi_progress,
            debug,
            persistence_provider,
            cache_provider,
            sqlite_db_url,
            postgres_db_name,
            postgres_user,
            postgres_password,
            postgres_hostname,
        ) => {
            result
        }
        _ = shutdown_signal => {
            // Graceful shutdown - just exit cleanly
            eprintln!("Shutdown complete.");
            std::process::exit(0);
        }
    }
}

/// Internal function that runs workflows (separated for signal handling)
async fn run_workflows_internal(
    workflows: Vec<PathBuf>,
    input: Option<String>,
    registry: Option<Vec<PathBuf>>,
    config: JackdawConfig,
    multi_progress: MultiProgress,
    debug: bool,
    persistence_provider: String,
    cache_provider: String,
    sqlite_db_url: Option<String>,
    postgres_db_name: Option<String>,
    postgres_user: Option<String>,
    postgres_password: Option<String>,
    postgres_hostname: Option<String>,
) -> Result<()> {
    // Set debug mode
    crate::output::set_debug_mode(debug);

    // Print banner (only in debug mode)
    if debug {
        println!(
            "{}\n",
            style("Jackdaw Serverless Workflow Runtime").bold().cyan()
        );
    }

    // Discover workflow files
    let workflow_files = discover_workflow_files(&workflows)?;

    if config.verbose {
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

    // Initialize persistence and cache based on provider selection
    if config.verbose || debug {
        println!("{} Initializing providers...", style("→").cyan());
        println!("  • Persistence: {}", persistence_provider);
        println!("  • Cache: {}", cache_provider);
        println!();
    }

    // Create persistence provider
    let persistence: Arc<dyn PersistenceProvider> = match persistence_provider.as_str() {
        "memory" => {
            // Use in-memory persistence provider (no files created)
            Arc::new(InMemoryPersistence::new())
        }
        "redb" => {
            let durable_db = config
                .durable_db
                .clone()
                .unwrap_or_else(|| PathBuf::from("workflow.db"));
            Arc::new(RedbPersistence::new(
                durable_db.to_str().unwrap_or("workflow.db"),
            )?)
        }
        "sqlite" => {
            let db_url = sqlite_db_url.as_ref().ok_or_else(|| Error::InvalidWorkflowFile {
                message: "SQLite persistence provider requires --sqlite-db-url parameter".to_string(),
            })?;
            Arc::new(SqlitePersistence::new(db_url).await?)
        }
        "postgres" => {
            let db_url = build_postgres_url(
                postgres_db_name.as_ref(),
                postgres_user.as_ref(),
                postgres_password.as_ref(),
                postgres_hostname.as_ref(),
            )?;
            Arc::new(PostgresPersistence::new(&db_url).await?)
        }
        _ => {
            return Err(Error::InvalidWorkflowFile {
                message: format!(
                    "Invalid persistence provider '{}'. Valid options: memory, redb, sqlite, postgres",
                    persistence_provider
                ),
            });
        }
    };

    // Create cache provider
    let cache: Arc<dyn CacheProvider> = match cache_provider.as_str() {
        "memory" => Arc::new(InMemoryCache::new()),
        "redb" => {
            let cache_db_path = config.cache_db.as_ref()
                .map(|p| p.to_str().unwrap_or("cache.db"))
                .unwrap_or("cache.db");
            let cache_persistence = Arc::new(RedbPersistence::new(cache_db_path)?);
            Arc::new(RedbCache::new(cache_persistence.db.clone())?)
        }
        "sqlite" => {
            let db_url = sqlite_db_url.as_ref().ok_or_else(|| Error::InvalidWorkflowFile {
                message: "SQLite cache provider requires --sqlite-db-url parameter".to_string(),
            })?;
            Arc::new(SqliteCache::new(db_url).await?)
        }
        "postgres" => {
            let db_url = build_postgres_url(
                postgres_db_name.as_ref(),
                postgres_user.as_ref(),
                postgres_password.as_ref(),
                postgres_hostname.as_ref(),
            )?;
            Arc::new(PostgresCache::new(&db_url).await?)
        }
        _ => {
            return Err(Error::InvalidWorkflowFile {
                message: format!(
                    "Invalid cache provider '{}'. Valid options: memory, redb, sqlite, postgres",
                    cache_provider
                ),
            });
        }
    };

    let engine = Arc::new(DurableEngine::new(persistence.clone(), cache.clone())?);

    // Register workflows from registry paths (if provided)
    if let Some(registry_paths) = registry {
        if config.verbose {
            println!(
                "{} Registering workflows from registry...",
                style("→").cyan()
            );
        }
        let registry_files = discover_workflow_files(&registry_paths)?;
        for workflow_path in &registry_files {
            let workflow_yaml = std::fs::read_to_string(workflow_path)?;
            let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)?;
            engine.register_workflow(workflow).await?;
            if config.verbose {
                println!("  • Registered workflow from {}", workflow_path.display());
            }
        }
        if config.verbose {
            println!();
        }
    }

    // Execute workflows
    if config.parallel && workflow_files.len() > 1 {
        // Parallel execution using futures::join_all
        if debug || config.verbose {
            multi_progress.println(format!(
                "{} Executing {} workflows in parallel...\n",
                style("→").cyan(),
                workflow_files.len()
            ))?;
        }

        let futures: Vec<_> = workflow_files
            .iter()
            .map(|workflow_path| {
                let engine_clone = engine.clone();
                let verbose = config.verbose;
                let path = workflow_path.clone();
                let input_clone = input.clone();
                let pb = multi_progress.add(ProgressBar::new_spinner());
                let style_result = ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg}")
                    .map_err(|e| Error::Progress {
                        source: std::io::Error::other(e.to_string()),
                    });

                async move {
                    let style = match style_result {
                        Ok(s) => s,
                        Err(e) => return (path, Err(e)),
                    };
                    pb.set_style(style);
                    pb.enable_steady_tick(std::time::Duration::from_millis(100));

                    let result = execute_workflow(
                        &path,
                        engine_clone,
                        Some(&pb),
                        verbose,
                        input_clone.as_ref(),
                    )
                    .await;
                    pb.finish_and_clear();
                    (path, result)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        // Print results
        if debug || config.verbose {
            multi_progress.println(format!("\n{}", style("Results:").bold().green()))?;
        }
        for (path, result) in results {
            match result {
                Ok((instance_id, output, workflow)) => {
                    if debug || config.verbose {
                        multi_progress.println(format!(
                            "\n{} {}",
                            style("✓").green(),
                            style(path.display()).bold()
                        ))?;
                    }

                    // Always output the final result as JSON (even in non-debug mode)
                    let filtered = filter_internal_fields(&output);
                    multi_progress.println(serde_json::to_string_pretty(&filtered)?)?;

                    // Visualization if requested
                    if config.visualize {
                        let viz_format = config.viz_format.as_deref().unwrap_or("svg");
                        let format = parse_diagram_format(viz_format)?;
                        let output_path = config.viz_output.as_deref();

                        multi_progress.println(format!(
                            "\n{} Generating visualization...",
                            style("→").cyan()
                        ))?;

                        let viz_tool = config.viz_tool.as_deref().unwrap_or("d2");
                        engine
                            .visualize_execution(
                                &workflow,
                                &instance_id,
                                output_path,
                                format,
                                viz_tool,
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
                    let error_msg = format!("{e}");
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
        if debug || config.verbose {
            multi_progress.println(format!(
                "{} Executing {} workflow(s)...\n",
                style("→").cyan(),
                workflow_files.len()
            ))?;
        }

        // Only show progress bars in debug/verbose mode
        let pb = if debug || config.verbose {
            let progress_bar = multi_progress.add(ProgressBar::new(workflow_files.len() as u64));
            progress_bar.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                    .map_err(|e| Error::Progress {
                        source: std::io::Error::other(e.to_string()),
                    })?
                    .progress_chars("#>-"),
            );
            Some(progress_bar)
        } else {
            None
        };

        for workflow_path in workflow_files {
            match execute_workflow(
                &workflow_path,
                engine.clone(),
                pb.as_ref(),
                config.verbose,
                input.as_ref(),
            )
            .await
            {
                Ok((instance_id, result, workflow)) => {
                    // Always output the final result as JSON (even in non-debug mode)
                    let filtered = filter_internal_fields(&result);
                    multi_progress.println(serde_json::to_string_pretty(&filtered)?)?;

                    // Visualization if requested
                    if config.visualize {
                        let viz_format = config.viz_format.as_deref().unwrap_or("svg");
                        let format = parse_diagram_format(viz_format)?;
                        let output_path = config.viz_output.as_deref();

                        multi_progress.println(format!(
                            "\n{} Generating visualization...",
                            style("→").cyan()
                        ))?;

                        let viz_tool = config.viz_tool.as_deref().unwrap_or("d2");
                        engine
                            .visualize_execution(
                                &workflow,
                                &instance_id,
                                output_path,
                                format,
                                viz_tool,
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
            if let Some(ref progress_bar) = pb {
                progress_bar.inc(1);
            }
        }

        if let Some(progress_bar) = pb {
            progress_bar.finish_with_message("All workflows completed");
        }
    }

    Ok(())
}
