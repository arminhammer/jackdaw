pub mod d2;
pub mod graphviz;

pub use self::d2::D2Provider;
pub use self::graphviz::GraphvizProvider;

use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Visualization error: {message}"))]
    Visualization { message: String },

    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("Tool not installed: {tool}\n{install_instructions}"))]
    ToolNotInstalled {
        tool: String,
        install_instructions: String,
    },

    #[snafu(display("Failed to spawn process '{command}': {source}"))]
    SpawnFailed {
        command: String,
        source: std::io::Error,
    },

    #[snafu(display("Failed to open stdin"))]
    StdinFailed,

    #[snafu(display("Failed to write to stdin: {source}"))]
    WriteStdinFailed { source: std::io::Error },

    #[snafu(display("Failed to wait for process '{command}': {source}"))]
    WaitFailed {
        command: String,
        source: std::io::Error,
    },

    #[snafu(display("Command '{command}' failed: {stderr}"))]
    CommandFailed { command: String, stderr: String },

    #[snafu(display("Output path required for {format:?} format"))]
    OutputPathRequired { format: DiagramFormat },

    #[snafu(display("Failed to execute '{command}': {source}"))]
    ExecuteFailed {
        command: String,
        source: std::io::Error,
    },

    #[snafu(display("Failed to create temporary directory: {source}"))]
    TempDirFailed { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Output format for diagram rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagramFormat {
    SVG,
    PNG,
    PDF,
    ASCII,
}

impl DiagramFormat {
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "svg" => Some(DiagramFormat::SVG),
            "png" => Some(DiagramFormat::PNG),
            "pdf" => Some(DiagramFormat::PDF),
            "txt" | "ascii" => Some(DiagramFormat::ASCII),
            _ => None,
        }
    }

    #[must_use]
    pub fn extension(&self) -> &'static str {
        match self {
            DiagramFormat::SVG => "svg",
            DiagramFormat::PNG => "png",
            DiagramFormat::PDF => "pdf",
            DiagramFormat::ASCII => "txt",
        }
    }

    #[must_use]
    pub fn is_terminal_output(&self) -> bool {
        matches!(self, DiagramFormat::ASCII)
    }
}

/// Execution state for a task in the workflow
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TaskExecutionState {
    /// Task has not been executed
    NotExecuted,
    /// Task executed successfully
    Success,
    /// Task execution failed
    Failed,
    /// Task is currently running
    Running,
}

/// Execution state information for workflow visualization
#[derive(Debug, Clone, Default)]
pub struct ExecutionState {
    /// Map of task name to its execution state
    pub task_states: HashMap<String, TaskExecutionState>,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn mark_success(&mut self, task_name: &str) {
        self.task_states
            .insert(task_name.to_string(), TaskExecutionState::Success);
    }

    #[allow(dead_code)]
    pub fn mark_failed(&mut self, task_name: &str) {
        self.task_states
            .insert(task_name.to_string(), TaskExecutionState::Failed);
    }

    #[allow(dead_code)]
    pub fn mark_running(&mut self, task_name: &str) {
        self.task_states
            .insert(task_name.to_string(), TaskExecutionState::Running);
    }
}

/// Common trait for workflow visualization providers
pub trait VisualizationProvider: Send + Sync + std::fmt::Debug {
    /// Get the name of the visualization tool (e.g., "graphviz", "d2")
    fn name(&self) -> &'static str;

    /// Generate diagram source code from a workflow definition
    ///
    /// # Arguments
    /// * `workflow` - The workflow to visualize
    /// * `execution_state` - Optional execution state to highlight completed/failed tasks
    ///
    /// # Errors
    /// Returns an error if source generation fails due to invalid workflow data,
    /// unsupported features, or internal provider errors.
    fn generate_source(
        &self,
        workflow: &WorkflowDefinition,
        execution_state: Option<&ExecutionState>,
    ) -> Result<String>;

    /// Render the diagram to a file or stdout
    ///
    /// # Arguments
    /// * `workflow` - The workflow to visualize
    /// * `output_path` - Path where the output file should be written (None for stdout/ASCII)
    /// * `format` - Output format (SVG, PNG, PDF, ASCII)
    /// * `execution_state` - Optional execution state to highlight completed/failed tasks
    ///
    /// # Errors
    /// Returns an error if rendering fails, the output path is invalid or missing for file formats,
    /// or if required external tools cannot be executed.
    fn render(
        &self,
        workflow: &WorkflowDefinition,
        output_path: Option<&Path>,
        format: DiagramFormat,
        execution_state: Option<&ExecutionState>,
    ) -> Result<()>;

    /// Check if the visualization tool is installed and available
    ///
    /// # Errors
    /// Returns an error if probing the tool’s availability fails (e.g., I/O or spawn errors).
    fn is_available(&self) -> Result<bool>;

    /// Get the version of the installed visualization tool
    ///
    /// # Errors
    /// Returns an error if the version cannot be determined (e.g., tool not found or command failure).
    #[allow(dead_code)]
    fn version(&self) -> Result<String>;
}
