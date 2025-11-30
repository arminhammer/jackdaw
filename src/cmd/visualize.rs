use clap::Parser;
use console::style;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::{ResultExt, prelude::*};
use std::path::PathBuf;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Invalid format '{}'. Valid formats: svg, png, pdf, ascii", format))]
    InvalidFormat { format: String },

    #[snafu(display("Failed to read workflow file '{}'", path.display()))]
    ReadWorkflow {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Failed to parse workflow file '{}'", path.display()))]
    ParseWorkflow {
        path: PathBuf,
        source: serde_yaml::Error,
    },

    #[snafu(display(
        "Output path (-o/--output) is required for {} format. Only ascii format can output to stdout.",
        format
    ))]
    MissingOutputPath { format: String },

    #[snafu(display("{message}"))]
    NotImplemented { message: String },
}

// use crate::durableengine::DurableEngine;
// use crate::providers::cache::RedbCache;
// use crate::providers::persistence::RedbPersistence;
use crate::providers::visualization::DiagramFormat;

#[derive(Parser, Debug)]
pub struct VisualizeArgs {
    /// Workflow file to visualize
    #[arg(required = true, value_name = "WORKFLOW")]
    pub workflow: PathBuf,

    /// Workflow instance ID to show execution state (optional)
    #[arg(short = 'i', long, value_name = "ID")]
    pub instance_id: Option<String>,

    /// Path to the durable persistence database (required if showing execution state)
    #[arg(short = 'd', long, default_value = "workflow.db", value_name = "PATH")]
    pub durable_db: PathBuf,

    /// Visualization tool to use (graphviz or d2)
    #[arg(short = 't', long, default_value = "graphviz", value_name = "TOOL")]
    pub tool: String,

    /// Output format (svg, png, pdf, ascii)
    #[arg(short = 'f', long, default_value = "svg", value_name = "FORMAT")]
    pub format: String,

    /// Output path (optional, defaults to stdout for ascii)
    #[arg(short = 'o', long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Enable verbose output
    #[arg(short = 'v', long)]
    pub verbose: bool,
}

/// Parse diagram format from string
fn parse_diagram_format(format_str: &str) -> Result<DiagramFormat, Error> {
    match format_str.to_lowercase().as_str() {
        "svg" => Ok(DiagramFormat::Svg),
        "png" => Ok(DiagramFormat::Png),
        "pdf" => Ok(DiagramFormat::Pdf),
        "ascii" => Ok(DiagramFormat::Ascii),
        _ => Err(Error::InvalidFormat {
            format: format_str.to_string(),
        }),
    }
}

/// Handle the visualize subcommand
pub async fn handle_visualize(args: VisualizeArgs) -> Result<(), Error> {
    if args.verbose {
        println!("{}\n", style("Workflow Visualization").bold().cyan());
    }

    // Read and parse workflow
    let workflow_yaml = std::fs::read_to_string(&args.workflow).context(ReadWorkflowSnafu {
        path: args.workflow.clone(),
    })?;

    let workflow: WorkflowDefinition =
        serde_yaml::from_str(&workflow_yaml).context(ParseWorkflowSnafu {
            path: args.workflow.clone(),
        })?;

    if args.verbose {
        println!(
            "{} Loaded workflow: {}",
            style("→").cyan(),
            workflow.document.name
        );
    }

    // Parse format
    let format = parse_diagram_format(&args.format)?;

    // Validate output path for non-ASCII formats
    if !matches!(format, DiagramFormat::Ascii) && args.output.is_none() {
        return Err(Error::MissingOutputPath {
            format: args.format.clone(),
        });
    }

    // // Initialize persistence and engine
    // let persistence = Arc::new(RedbPersistence::new(
    //     args.durable_db.to_str().unwrap_or("workflow.db"),
    // )?);
    // let cache = Arc::new(RedbCache::new(persistence.db.clone())?);
    // let engine = Arc::new(DurableEngine::new(persistence, cache)?);

    // Generate visualization
    // if args.verbose {
    //     println!(
    //         "{} Generating {} visualization using {}...",
    //         style("→").cyan(),
    //         args.format,
    //         args.tool
    //     );
    // }

    // let output_path = args.output.as_deref();
    // let instance_id = args.instance_id.as_deref().unwrap_or("");

    // engine
    //     .visualize_execution(&workflow, instance_id, output_path, format, &args.tool)
    //     .await?;

    // if let Some(output_path) = output_path {
    //     println!(
    //         "{} Visualization saved to: {}",
    //         style("✓").green(),
    //         output_path.display()
    //     );
    // }

    Err(Error::NotImplemented {
        message: "Error handling not yet implemented in visualize command".to_string(),
    })
}
