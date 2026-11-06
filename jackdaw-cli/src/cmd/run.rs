use clap::Parser;
use console::style;
use indicatif::MultiProgress;
use jackdaw::Engine;
use jackdaw::api::Workflow;
use jackdaw::dsl::workflow::WorkflowDefinition;
use jackdaw::runner::{
    Env, Headless, Json, Kind, Lines, Reporter, RunOptions, Runner, RunnerArgs, Terminal,
};
use serde_json::Value;
use snafu::prelude::*;
use std::path::{Path, PathBuf};

use super::providers::ProviderArgs;
use crate::config::JackdawConfig;
use crate::visualization::{self, DiagramFormat, ExecutionState};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Invalid workflow file: {message}"))]
    InvalidWorkflowFile { message: String },

    #[snafu(display("Path error: {message}"))]
    Path { message: String },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("YAML parsing error: {source}"))]
    // Boxed: serde-saphyr's error carries the source snippet and is large.
    Yaml { source: Box<serde_saphyr::Error> },

    #[snafu(display("JSON serialization error: {source}"))]
    Json { source: serde_json::Error },

    #[snafu(display("{source}"))]
    Engine { source: jackdaw::Error },

    #[snafu(display("{source}"))]
    Run { source: jackdaw::runner::Error },

    #[snafu(display("Workflow did not finish within {seconds}s"))]
    Timeout { seconds: u64 },

    #[snafu(display("Visualization error: {source}"))]
    Visualization { source: visualization::Error },

    #[snafu(display("Cache error: {source}"))]
    Cache { source: jackdaw::cache::Error },

    #[snafu(display("Persistence error: {source}"))]
    Persistence { source: jackdaw::persistence::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<jackdaw::runner::Error> for Error {
    fn from(source: jackdaw::runner::Error) -> Self {
        Error::Run { source }
    }
}

impl From<jackdaw::Error> for Error {
    fn from(source: jackdaw::Error) -> Self {
        Error::Engine { source }
    }
}

impl From<visualization::Error> for Error {
    fn from(source: visualization::Error) -> Self {
        Error::Visualization { source }
    }
}

/// How long a run may take before the CLI gives up on it.
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

impl From<jackdaw::cache::Error> for Error {
    fn from(source: jackdaw::cache::Error) -> Self {
        Error::Cache { source }
    }
}

impl From<jackdaw::persistence::Error> for Error {
    fn from(source: jackdaw::persistence::Error) -> Self {
        Error::Persistence { source }
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { source }
    }
}

impl From<serde_saphyr::Error> for Error {
    fn from(source: serde_saphyr::Error) -> Self {
        Error::Yaml {
            source: Box::new(source),
        }
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

    /// Enable debug mode (show what each step returned, and each task's input and cache use)
    #[arg(long)]
    pub debug: bool,

    #[command(flatten)]
    pub providers: ProviderArgs,

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

    /// Run under this id: running again with a durable store continues it, or returns its result if it finished
    #[arg(long, value_name = "ID")]
    pub id: Option<String>,

    #[command(flatten)]
    pub runner: RunnerArgs,
}

impl RunArgs {
    /// Merge CLI arguments with config file settings
    /// CLI arguments take precedence over config file settings
    pub fn merge_with_config(&self, config: JackdawConfig) -> JackdawConfig {
        JackdawConfig {
            durable_db: self.durable_db.clone().or(config.durable_db),
            cache_db: self.cache_db.clone().or(config.cache_db),
            parallel: self.parallel || config.parallel,
            verbose: self.verbose || config.verbose,
            visualize: self.visualize || config.visualize,
            viz_tool: self.viz_tool.clone().or(config.viz_tool),
            viz_format: self.viz_format.clone().or(config.viz_format),
            viz_output: self.viz_output.clone().or(config.viz_output),
        }
    }
}

/// The workflow files in `paths`: the files themselves, and the `.yaml` and `.yml` files in each directory.
fn discover_workflow_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_file() {
            if !is_workflow_file(path) {
                return Err(Error::InvalidWorkflowFile {
                    message: format!(
                        "File {} is not a valid workflow file (.yaml or .yml)",
                        path.display()
                    ),
                });
            }
            files.push(path.clone());
        } else if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?.path();
                if entry.is_file() && is_workflow_file(&entry) {
                    files.push(entry);
                }
            }
        } else {
            return Err(Error::Path {
                message: format!("Path {} does not exist", path.display()),
            });
        }
    }
    if files.is_empty() {
        return Err(Error::Path {
            message: "No workflow files found in the provided paths".to_string(),
        });
    }
    Ok(files)
}

fn is_workflow_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
}

fn load_workflow(path: &Path) -> Result<WorkflowDefinition> {
    Ok(serde_saphyr::from_str(&std::fs::read_to_string(path)?)?)
}

/// The workflow's input: JSON given on the command line, or the JSON file it names. `{}` if none was given.
fn parse_input(input: Option<&String>) -> Result<Value> {
    let Some(text) = input else {
        return Ok(serde_json::json!({}));
    };
    if let Ok(json) = serde_json::from_str(text) {
        return Ok(json);
    }
    let path = PathBuf::from(text);
    if path.exists() {
        return Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?);
    }
    Err(Error::InvalidWorkflowFile {
        message: format!("Input '{text}' is neither valid JSON nor a valid file path"),
    })
}

fn parse_diagram_format(format: &str) -> Result<DiagramFormat> {
    match format.to_lowercase().as_str() {
        "svg" => Ok(DiagramFormat::Svg),
        "png" => Ok(DiagramFormat::Png),
        "pdf" => Ok(DiagramFormat::Pdf),
        "ascii" => Ok(DiagramFormat::Ascii),
        _ => Err(Error::InvalidWorkflowFile {
            message: format!("Invalid format '{format}'. Valid formats: svg, png, pdf, ascii"),
        }),
    }
}

/// The output without the engine's internal descriptor fields (those starting with `__`).
fn filter_internal_fields(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !key.starts_with("__"))
                .map(|(key, value)| (key.clone(), filter_internal_fields(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(filter_internal_fields).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => {
            other.clone()
        }
    }
}

/// Prints to standard output without breaking the progress bars.
pub fn out(display: &MultiProgress, text: impl std::fmt::Display) {
    display.suspend(|| println!("{text}"));
}

/// Prints to standard error without breaking the progress bars.
pub fn err(display: &MultiProgress, text: impl std::fmt::Display) {
    display.suspend(|| eprintln!("{text}"));
}

/// One workflow to run.
struct Job {
    path: PathBuf,
    definition: WorkflowDefinition,
    runnable: Workflow<Value, Value>,
    id: String,
}

/// The reporter for one run: what the flags and the terminal call for, drawing into the shared `display`. Runs side by
/// side are labelled with their workflow's name.
fn reporter(args: &RunArgs, display: &MultiProgress, label: Option<&str>) -> Box<dyn Reporter> {
    match args.runner.kind(Env::detect()) {
        Kind::Terminal => {
            let terminal = Terminal::in_group(display).outputs(args.debug);
            Box::new(match label {
                Some(label) => terminal.label(label),
                None => terminal,
            })
        }
        Kind::Lines => Box::new(Lines::stderr().outputs(args.debug)),
        Kind::Json => Box::new(Json::stderr()),
        Kind::Headless | _ => Box::new(Headless),
    }
}

/// Handle the run subcommand, stopping cleanly on Ctrl+C or SIGTERM.
pub async fn handle_run(
    args: RunArgs,
    config: JackdawConfig,
    display: MultiProgress,
) -> Result<()> {
    tokio::select! {
        result = run_workflows(&args, config, &display) => result,
        signal = shutdown_signal() => {
            err(&display, format!("\nReceived {signal}, shutting down gracefully...\nShutdown complete."));
            std::process::exit(0);
        }
    }
}

/// Waits for Ctrl+C (and SIGTERM on Unix), and says which it was.
async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let (Ok(mut int), Ok(mut term)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) {
            return tokio::select! {
                _ = int.recv() => "SIGINT (Ctrl+C)",
                _ = term.recv() => "SIGTERM",
            };
        }
    }
    match tokio::signal::ctrl_c().await {
        Ok(()) => "Ctrl+C",
        Err(_) => std::future::pending().await,
    }
}

async fn run_workflows(
    args: &RunArgs,
    config: JackdawConfig,
    display: &MultiProgress,
) -> Result<()> {
    let config = args.merge_with_config(config);
    let verbose = config.verbose || args.debug;
    let files = discover_workflow_files(&args.workflows)?;
    if args.id.is_some() && files.len() > 1 {
        return Err(Error::InvalidWorkflowFile {
            message: "--id names one run, so it needs a single workflow".to_string(),
        });
    }
    if verbose {
        err(
            display,
            format!("{} {} workflow file(s)", style("→").cyan(), files.len()),
        );
    }

    let persistence = args.providers.persistence(&config).await?;
    let cache = args.providers.cache(&config).await?;
    let mut builder = Engine::builder().persistence(persistence).cache(cache);
    for path in
        discover_workflow_files(args.registry.as_deref().unwrap_or_default()).or_else(|e| {
            // No registry is fine; a registry that does not exist is not.
            if args.registry.is_none() {
                Ok(Vec::new())
            } else {
                Err(e)
            }
        })?
    {
        builder = builder.yaml_workflow(&Workflow::<Value, Value>::from_definition(load_workflow(
            &path,
        )?));
        if verbose {
            err(display, format!("  registered {}", path.display()));
        }
    }
    let engine = builder.build()?;

    let input = parse_input(args.input.as_ref())?;
    let jobs = files
        .into_iter()
        .map(|path| {
            let definition = load_workflow(&path)?;
            let id = args.id.clone().unwrap_or_else(|| {
                format!("{}/{}", definition.document.name, uuid::Uuid::new_v4())
            });
            let runnable = Workflow::<Value, Value>::from_definition(definition.clone());
            Ok(Job {
                path,
                definition,
                runnable,
                id,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let many = jobs.len() > 1;
    let run = |job| run_job(&engine, job, input.clone(), args, display, many);
    let outputs = if config.parallel && many {
        futures::future::join_all(jobs.iter().map(run)).await
    } else {
        let mut outputs = Vec::new();
        for job in &jobs {
            let output = run(job).await;
            let failed = output.is_err();
            outputs.push(output);
            if failed {
                break;
            }
        }
        outputs
    };

    for (job, output) in jobs.iter().zip(outputs) {
        match output {
            Ok(output) => {
                out(
                    display,
                    serde_json::to_string_pretty(&filter_internal_fields(&output))?,
                );
                if config.visualize {
                    visualize(&engine, job, &config, display).await?;
                }
            }
            Err(error) => {
                err(
                    display,
                    format!(
                        "{} {} - {}",
                        style("✗").red(),
                        style(job.path.display()).bold(),
                        style(&error).red()
                    ),
                );
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Runs one workflow on the engine, showing it as the flags and the terminal call for. Runs side by side are labelled.
async fn run_job(
    engine: &Engine,
    job: &Job,
    input: Value,
    args: &RunArgs,
    display: &MultiProgress,
    many: bool,
) -> Result<Value> {
    let label = many.then_some(job.definition.document.name.as_str());
    let options = RunOptions::new(&job.id).restart(args.runner.restart);
    let mut runner = Runner::new(engine).reporter(reporter(args, display, label));
    // A YAML workflow's own deadline is its `timeout`, so the CLI's limit is a limit on waiting.
    tokio::time::timeout(RUN_TIMEOUT, runner.run(&job.runnable, input, options))
        .await
        .map_err(|_| Error::Timeout {
            seconds: RUN_TIMEOUT.as_secs(),
        })?
        .map_err(Error::from)
}

/// Draws the workflow with what the run did, as the `--visualize` flags ask.
async fn visualize(
    engine: &Engine,
    job: &Job,
    config: &JackdawConfig,
    display: &MultiProgress,
) -> Result<()> {
    let format = parse_diagram_format(config.viz_format.as_deref().unwrap_or("svg"))?;
    let output_path = config.viz_output.as_deref();
    err(
        display,
        format!("\n{} Generating visualization...", style("→").cyan()),
    );
    let history = engine.history(&job.id).await?;
    visualization::render(
        config.viz_tool.as_deref().unwrap_or("d2"),
        &job.definition,
        output_path,
        format,
        Some(&ExecutionState::from_history(&history)),
    )?;
    if let Some(path) = output_path {
        err(
            display,
            format!(
                "{} Visualization saved to: {}",
                style("✓").green(),
                path.display()
            ),
        );
    }
    Ok(())
}
