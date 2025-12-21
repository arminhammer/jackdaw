use clap::Parser;
use indicatif::MultiProgress;
use snafu::prelude::*;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod cache;
mod cmd;
mod config;
mod container;
mod context;
mod descriptors;
mod durableengine;
mod executionhistory;
mod executor;
mod expressions;
mod listeners;
pub mod output;
mod persistence;
mod providers;
mod task_ext;
pub mod task_output;
mod workflow;

use cmd::{RunArgs, ValidateArgs, VisualizeArgs, handle_run, handle_validate, handle_visualize};
use config::JackdawConfig;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Run error: {source}"))]
    Run { source: cmd::run::Error },

    #[snafu(display("Validate error: {source}"))]
    Validate { source: cmd::validate::Error },

    #[snafu(display("Visualization error: {source}"))]
    Visualize { source: cmd::visualize::Error },
}

#[derive(Parser, Debug)]
#[command(name = "jackdaw")]
#[command(author = "Armin Graf")]
#[command(version = "1.0.0")]
#[command(about = "A durable, cached, graph-based execution engine for Serverless Workflows", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Parser, Debug)]
enum Commands {
    /// Execute workflow(s)
    Run(RunArgs),
    /// Validate workflow(s) without executing
    Validate(ValidateArgs),
    /// Visualize workflow structure and execution state
    Visualize(VisualizeArgs),
}

/// Initialize tracing/logging with indicatif integration
fn init_tracing(verbose: bool) {
    let indicatif_layer = tracing_indicatif::IndicatifLayer::new();

    let filter_layer = if verbose {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(indicatif_layer)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cli = Cli::parse();

    // Load configuration from file, env vars, and defaults
    let global_config = JackdawConfig::load().unwrap_or_default();

    match cli.command {
        Commands::Run(args) => {
            // Extract workflows, input, registry, and debug flag before merging
            let workflows = args.workflows.clone();
            let input = args.input.clone();
            let registry = args.registry.clone();
            let debug = args.debug;

            // Merge CLI args with config (CLI takes precedence)
            let config = args.merge_with_config(global_config);

            // Initialize tracing/logging with indicatif bridge
            init_tracing(config.verbose);

            // Initialize MultiProgress for coordinating progress bars and logs/traces
            let multi_progress = MultiProgress::new();

            handle_run(workflows, input, registry, config, multi_progress, debug)
                .await
                .context(RunSnafu)
        }
        Commands::Validate(args) => {
            // Initialize tracing/logging with indicatif bridge
            init_tracing(args.verbose);

            handle_validate(args).await.context(ValidateSnafu)
        }
        Commands::Visualize(args) => {
            // Initialize tracing/logging with indicatif bridge
            init_tracing(args.verbose);

            handle_visualize(args).await.context(VisualizeSnafu)
        }
    }
}
