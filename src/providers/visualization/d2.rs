use serverless_workflow_core::models::task::TaskDefinition;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::fmt::Write;
use std::path::Path;
use std::process::Command;

use super::{
    CommandFailedSnafu, DiagramFormat, ExecuteFailedSnafu, ExecutionState, OutputPathRequiredSnafu,
    Result, TaskExecutionState, TempDirFailedSnafu, ToolNotInstalledSnafu, VisualizationProvider,
};

const D2: &str = "d2";

#[derive(Debug, Default)]
pub struct D2Provider {
    /// Path to d2 executable (default: "d2" from PATH)
    d2_path: String,
    /// D2 theme to use (e.g., "0", "200", etc.)
    theme: Option<String>,
}

impl D2Provider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            d2_path: D2.to_string(),
            theme: None,
        }
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn with_d2_path(mut self, path: String) -> Self {
        self.d2_path = path;
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn with_theme(mut self, theme: String) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Generate D2 source for a workflow with optional execution state
    #[allow(clippy::unused_self)]
    fn workflow_to_d2(
        &self,
        workflow: &WorkflowDefinition,
        execution_state: Option<&ExecutionState>,
    ) -> String {
        let mut d2 = String::new();

        // Direction and metadata
        d2.push_str("direction: down\n\n");
        writeln!(d2, "# Workflow: {}", workflow.document.name).unwrap();
        writeln!(d2, "# Version: {}\n", workflow.document.version).unwrap();

        // Start node
        d2.push_str("Start: {\n");
        d2.push_str("  shape: circle\n");
        d2.push_str("  style.fill: \"#90EE90\"\n");
        d2.push_str("}\n\n");

        // Collect all task names in order
        let mut task_names = Vec::new();
        for entry in &workflow.do_.entries {
            for name in entry.keys() {
                task_names.push(name.clone());
            }
        }

        // Task nodes
        for entry in &workflow.do_.entries {
            for (name, task) in entry {
                let mut style = Self::task_style_d2(task);
                let label = Self::task_label(name, task);

                // Override style based on execution state
                if let Some(state) = execution_state
                    && let Some(task_state) = state.task_states.get(name)
                {
                    let color = match task_state {
                        TaskExecutionState::Success => "#90EE90", // Green
                        TaskExecutionState::Failed => "#FF6B6B",  // Red
                        TaskExecutionState::Running => "#FFD700", // Gold
                        TaskExecutionState::NotExecuted => {
                            // Keep default style
                            ""
                        }
                    };

                    if !color.is_empty() {
                        style = format!(
                            "  shape: rectangle\n  style.fill: \"{color}\"\n  style.border-radius: 8\n"
                        );
                    }
                }
                writeln!(d2, "\"{name}\": {{").unwrap();
                writeln!(d2, "  label: \"{label}\"").unwrap();
                d2.push_str(&style);
                d2.push_str(&style);
                d2.push_str("}\n\n");
            }
        }

        // End node
        d2.push_str("End: {\n");
        d2.push_str("  shape: circle\n");
        d2.push_str("  style.fill: \"#FFB6C1\"\n");
        d2.push_str("  style.double-border: true\n");
        d2.push_str("}\n\n");

        // Connections - build sequential flow
        if task_names.is_empty() {
            // Empty workflow
            d2.push_str("Start -> End\n");
        } else {
            // Start to first task
            writeln!(d2, "Start -> \"{}\"", task_names[0]).unwrap();

            // Sequential flow between tasks
            for i in 0..task_names.len() - 1 {
                writeln!(d2, "\"{}\" -> \"{}\"", task_names[i], task_names[i + 1]).unwrap();
            }

            // Last task to end
            writeln!(d2, "\"{}\" -> End", task_names[task_names.len() - 1]).unwrap();
        }

        d2
    }

    /// Determine node style for D2 based on task type
    fn task_style_d2(task: &TaskDefinition) -> String {
        let (shape, color) = match task {
            TaskDefinition::Call(_) => ("rectangle", "#87CEEB"),
            TaskDefinition::Run(_) => ("rectangle", "#DDA0DD"),
            TaskDefinition::Set(_) => ("rectangle", "#F0E68C"),
            TaskDefinition::Switch(_) => ("diamond", "#FFD700"),
            TaskDefinition::Fork(_) => ("parallelogram", "#FFA07A"),
            TaskDefinition::For(_) => ("hexagon", "#98FB98"),
            TaskDefinition::Try(_) => ("rectangle", "#FFE4B5"),
            TaskDefinition::Listen(_) => ("rectangle", "#E0BBE4"),
            TaskDefinition::Emit(_) => ("rectangle", "#FFDAB9"),
            TaskDefinition::Wait(_) => ("oval", "#D3D3D3"),
            TaskDefinition::Raise(_) => ("rectangle", "#FF6B6B"),
            TaskDefinition::Do(_) => ("rectangle", "#B0C4DE"),
        };

        format!("  shape: {shape}\n  style.fill: \"{color}\"\n  style.border-radius: 8\n")
    }

    /// Generate human-readable label for a task
    fn task_label(name: &str, task: &TaskDefinition) -> String {
        let task_type = match task {
            TaskDefinition::Call(_) => "Call",
            TaskDefinition::Run(_) => "Run",
            TaskDefinition::Set(_) => "Set",
            TaskDefinition::Switch(_) => "Switch",
            TaskDefinition::Fork(_) => "Fork",
            TaskDefinition::For(_) => "For",
            TaskDefinition::Try(_) => "Try",
            TaskDefinition::Listen(_) => "Listen",
            TaskDefinition::Emit(_) => "Emit",
            TaskDefinition::Wait(_) => "Wait",
            TaskDefinition::Raise(_) => "Raise",
            TaskDefinition::Do(_) => "Do",
        };
        format!("{task_type}\\n{name}")
    }
}

impl VisualizationProvider for D2Provider {
    fn name(&self) -> &'static str {
        D2
    }

    fn generate_source(
        &self,
        workflow: &WorkflowDefinition,
        execution_state: Option<&ExecutionState>,
    ) -> Result<String> {
        Ok(self.workflow_to_d2(workflow, execution_state))
    }

    fn render(
        &self,
        workflow: &WorkflowDefinition,
        output_path: Option<&Path>,
        format: DiagramFormat,
        execution_state: Option<&ExecutionState>,
    ) -> Result<()> {
        // Check if d2 is available
        if !self.is_available()? {
            return ToolNotInstalledSnafu {
                tool: "D2".to_string(),
                install_instructions: "Install with:\n\
                   - All platforms: curl -fsSL https://d2lang.com/install.sh | sh -s --\n\
                   - Or download from: https://github.com/terrastruct/d2/releases"
                    .to_string(),
            }
            .fail();
        }

        // Generate D2 source
        let d2_source = self.generate_source(workflow, execution_state)?;

        // D2 requires file input
        let temp_dir = tempfile::tempdir().context(TempDirFailedSnafu)?;
        let temp_source = temp_dir.path().join("workflow.d2");
        std::fs::write(&temp_source, d2_source).context(super::IoSnafu)?;

        if format == DiagramFormat::Ascii {
            // D2's --sketch flag outputs ASCII to stdout
            let output = Command::new(&self.d2_path)
                .arg("--sketch")
                .arg(&temp_source)
                .output()
                .context(ExecuteFailedSnafu {
                    command: "d2 --sketch",
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return CommandFailedSnafu {
                    command: "d2 --sketch".to_string(),
                    stderr: stderr.to_string(),
                }
                .fail();
            }

            let ascii_art = String::from_utf8_lossy(&output.stdout);

            if let Some(path) = output_path {
                std::fs::write(path, ascii_art.as_bytes()).context(super::IoSnafu)?;
            } else {
                print!("{ascii_art}");
            }
        } else {
            // SVG, PNG, PDF
            let output_path =
                output_path.ok_or_else(|| OutputPathRequiredSnafu { format }.build())?;

            let mut cmd = Command::new(&self.d2_path);

            // Add theme if specified
            if let Some(ref theme) = self.theme {
                cmd.arg("--theme").arg(theme);
            }

            cmd.arg(&temp_source).arg(output_path);

            let output = cmd.output().context(ExecuteFailedSnafu { command: "d2" })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return CommandFailedSnafu {
                    command: "d2".to_string(),
                    stderr: stderr.to_string(),
                }
                .fail();
            }
        }

        Ok(())
    }

    fn is_available(&self) -> Result<bool> {
        Ok(Command::new(&self.d2_path)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false))
    }

    fn version(&self) -> Result<String> {
        let output = Command::new(&self.d2_path)
            .arg("--version")
            .output()
            .context(ExecuteFailedSnafu {
                command: "d2 --version",
            })?;

        if !output.status.success() {
            return CommandFailedSnafu {
                command: "d2 --version".to_string(),
                stderr: "Command failed".to_string(),
            }
            .fail();
        }

        let version_str = String::from_utf8_lossy(&output.stdout);
        Ok(version_str.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serverless_workflow_core::models::workflow::WorkflowDefinitionMetadata;

    fn create_test_workflow() -> WorkflowDefinition {
        WorkflowDefinition::new(WorkflowDefinitionMetadata {
            name: "test-workflow".to_string(),
            version: "1.0.0".to_string(),
            namespace: "default".to_string(),
            ..Default::default()
        })
    }

    #[test]
    fn test_d2_source_generation() {
        let workflow = create_test_workflow();
        let provider = D2Provider::new();
        let source = provider.generate_source(&workflow, None).unwrap();

        assert!(source.contains("Start"));
        assert!(source.contains("End"));
        assert!(source.contains("->"));
        assert!(source.contains("test-workflow"));
    }

    #[test]
    fn test_is_available() {
        let provider = D2Provider::new();
        // This test just checks it doesn't panic
        let _ = provider.is_available();
    }
}
