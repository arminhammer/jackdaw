use clap::Parser;
use snafu::prelude::*;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod cache;
mod cmd;
mod context;
mod descriptors;
// mod durableengine;
mod executionhistory;
mod executor;
mod expressions;
mod listeners;
pub mod output;
mod persistence;
mod providers;
mod workflow;

// use cmd::{RunArgs, VisualizeArgs, handle_run, handle_visualize};
use cmd::{VisualizeArgs, handle_visualize};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Visualization error: {source}"))]
    Visualize { source: cmd::visualize::Error },
}

#[derive(Parser, Debug)]
#[command(name = "mooose")]
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
    // Run(RunArgs),
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

    match cli.command {
        // Commands::Run(args) => {
        //     // Initialize tracing/logging with indicatif bridge
        //     // init_tracing(args.verbose);

        //     // Initialize MultiProgress for coordinating progress bars and logs/traces
        //     let multi_progress = MultiProgress::new();

        //     handle_run(args, multi_progress).await
        // }
        Commands::Visualize(args) => {
            // Initialize tracing/logging with indicatif bridge
            init_tracing(args.verbose);

            handle_visualize(args).await.context(VisualizeSnafu)
        }
    }
}
