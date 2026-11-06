use clap::Parser;
use indicatif::{MultiProgress, ProgressDrawTarget};
use snafu::prelude::*;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod cmd;
mod config;
mod trace;
mod visualization;

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
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "A durable, cached, graph-based execution engine for the Open Workflow DSL (formerly Serverless Workflow)", long_about = None)]
#[command(help_template = "\
{before-help}{name} {version}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}
")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Parser, Debug)]
enum Commands {
    /// Execute workflow(s)
    Run(RunArgs),
    /// Validate workflow(s) without executing
    Validate(ValidateArgs),
    /// Visualize workflow structure and execution state
    Visualize(VisualizeArgs),
}

/// Sets up the terminal output: progress bars, the library's YAML trace ([`trace`]; its task-by-task part only with
/// `debug`) and log lines on stderr (warnings, or everything at `verbose`; `RUST_LOG` overrides).
fn init_tracing(verbose: bool, debug: bool, display: &MultiProgress) {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::filter::{LevelFilter, Targets};

    let trace_level = if debug {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };
    let trace_layer = trace::YamlTrace(display.clone())
        .with_filter(Targets::new().with_target(trace::TARGET, trace_level));

    let default_logs = if verbose { "debug" } else { "warn" };
    let log_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_logs))
        .add_directive(
            format!("{}=off", trace::TARGET)
                .parse()
                .unwrap_or_else(|_| LevelFilter::OFF.into()),
        );
    let log_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(log_filter);

    tracing_subscriber::registry()
        .with(trace_layer)
        .with(log_layer)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cli = Cli::parse();

    // Load configuration from file, env vars, and defaults
    let global_config = JackdawConfig::load().unwrap_or_default();

    // Progress bars, trace lines and results share this display, so they do not overwrite each other.
    let display = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());

    match cli.command {
        Commands::Run(args) => {
            init_tracing(args.verbose || global_config.verbose, args.debug, &display);
            handle_run(args, global_config, display)
                .await
                .context(RunSnafu)
        }
        Commands::Validate(args) => {
            init_tracing(args.verbose, false, &display);
            handle_validate(args).await.context(ValidateSnafu)
        }
        Commands::Visualize(args) => {
            init_tracing(args.verbose, false, &display);
            handle_visualize(args).await.context(VisualizeSnafu)
        }
    }
}
