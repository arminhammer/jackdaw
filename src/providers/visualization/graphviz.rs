use serverless_workflow_core::models::task::TaskDefinition;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use super::{
    CommandFailedSnafu, DiagramFormat, ExecuteFailedSnafu, ExecutionState, OutputPathRequiredSnafu,
    Result, SpawnFailedSnafu, StdinFailedSnafu, TaskExecutionState, ToolNotInstalledSnafu,
    VisualizationProvider, WaitFailedSnafu, WriteStdinFailedSnafu,
};

#[derive(Debug, Default)]
pub struct GraphvizProvider {
    /// Path to dot executable (default: "dot" from PATH)
    dot_path: String,
}

impl GraphvizProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            dot_path: "dot".to_string(),
        }
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn with_dot_path(mut self, path: String) -> Self {
        self.dot_path = path;
        self
    }

    /// Generate DOT source for a workflow with optional execution state
    #[allow(clippy::unused_self)]
    fn workflow_to_dot(
        &self,
        workflow: &WorkflowDefinition,
        execution_state: Option<&ExecutionState>,
    ) -> String {
        let mut dot = String::new();

        // Graph header
        let _ = writeln!(&mut dot, "digraph \"{}\" {{", workflow.document.name);
        dot.push_str("  rankdir=TB;\n");
        dot.push_str("  node [shape=box, style=\"rounded,filled\", fontname=\"Helvetica\"];\n");
        dot.push_str("  edge [fontname=\"Helvetica\", fontsize=10];\n\n");

        // Start node
        dot.push_str("  start [shape=circle, label=\"Start\", fillcolor=\"#90EE90\"];\n");

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
                let (shape, mut color) = Self::task_style(task);
                let label = Self::task_label(name, task);

                // Override color based on execution state
                if let Some(state) = execution_state
                    && let Some(task_state) = state.task_states.get(name)
                {
                    color = match task_state {
                        TaskExecutionState::Success => "#90EE90", // Green
                        TaskExecutionState::Failed => "#FF6B6B",  // Red
                        TaskExecutionState::Running => "#FFD700", // Gold
                        TaskExecutionState::NotExecuted => color, // Default
                    };
                }
                let _ = writeln!(
                    &mut dot,
                    "  \"{name}\" [label=\"{label}\", shape={shape}, fillcolor=\"{color}\"];"
                );
            }
        }

        // End node
        dot.push_str("  end [shape=doublecircle, label=\"End\", fillcolor=\"#FFB6C1\"];\n\n");

        // Edges - build sequential flow
        if task_names.is_empty() {
            // Empty workflow
            dot.push_str("  start -> end;\n");
        } else {
            // Start to first task
            // Start to first task
            if let Some(first) = task_names.first() {
                let _ = writeln!(&mut dot, "  start -> \"{first}\";");
            }
            // Sequential flow between tasks
            // Sequential flow between tasks
            for pair in task_names.windows(2) {
                if let [from, to] = pair {
                    let _ = writeln!(&mut dot, "  \"{from}\" -> \"{to}\";");
                }
            }
            // Last task to end
            // Last task to end
            if let Some(last) = task_names.last() {
                let _ = writeln!(&mut dot, "  \"{last}\" -> end;");
            }
        }

        dot.push_str("}\n");
        dot
    }

    /// Determine node style based on task type
    fn task_style(task: &TaskDefinition) -> (&str, &str) {
        match task {
            TaskDefinition::Call(_) => ("box", "#87CEEB"), // Sky blue
            TaskDefinition::Run(_) => ("box", "#DDA0DD"),  // Plum
            TaskDefinition::Set(_) => ("box", "#F0E68C"),  // Khaki
            TaskDefinition::Switch(_) => ("diamond", "#FFD700"), // Gold
            TaskDefinition::Fork(_) => ("parallelogram", "#FFA07A"), // Light salmon
            TaskDefinition::For(_) => ("hexagon", "#98FB98"), // Pale green
            TaskDefinition::Try(_) => ("box", "#FFE4B5"),  // Moccasin
            TaskDefinition::Listen(_) => ("invtrapezium", "#E0BBE4"), // Lavender
            TaskDefinition::Emit(_) => ("trapezium", "#FFDAB9"), // Peach
            TaskDefinition::Wait(_) => ("octagon", "#D3D3D3"), // Light gray
            TaskDefinition::Raise(_) => ("tripleoctagon", "#FF6B6B"), // Red
            TaskDefinition::Do(_) => ("box", "#B0C4DE"),   // Light steel blue
        }
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
        format!("{task_type}: {name}")
    }
}

impl VisualizationProvider for GraphvizProvider {
    fn name(&self) -> &'static str {
        "graphviz"
    }

    fn generate_source(
        &self,
        workflow: &WorkflowDefinition,
        execution_state: Option<&ExecutionState>,
    ) -> Result<String> {
        Ok(self.workflow_to_dot(workflow, execution_state))
    }

    fn render(
        &self,
        workflow: &WorkflowDefinition,
        output_path: Option<&Path>,
        format: DiagramFormat,
        execution_state: Option<&ExecutionState>,
    ) -> Result<()> {
        // Check if graphviz is available
        if !self.is_available()? {
            return ToolNotInstalledSnafu {
                tool: "Graphviz (dot)".to_string(),
                install_instructions: "Install with:\n\
                   - Ubuntu/Debian: sudo apt-get install graphviz\n\
                   - macOS: brew install graphviz\n\
                   - Windows: choco install graphviz"
                    .to_string(),
            }
            .fail();
        }

        // Generate DOT source
        let dot_source = self.generate_source(workflow, execution_state)?;

        if format == DiagramFormat::Ascii {
            // Check for graph-easy
            let graph_easy_available = Command::new("graph-easy")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if !graph_easy_available {
                return ToolNotInstalledSnafu {
                    tool: "graph-easy".to_string(),
                    install_instructions: "Install with:\n\
                       - Ubuntu/Debian: sudo apt-get install libgraph-easy-perl\n\
                       - macOS: brew install graph-easy\n\
                       - CPAN: cpan Graph::Easy"
                        .to_string(),
                }
                .fail();
            }

            let mut cmd = Command::new("graph-easy")
                .arg("--from=dot")
                .arg("--as=boxart")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context(SpawnFailedSnafu {
                    command: "graph-easy",
                })?;

            cmd.stdin
                .as_mut()
                .ok_or(StdinFailedSnafu.build())?
                .write_all(dot_source.as_bytes())
                .context(WriteStdinFailedSnafu)?;

            let output = cmd.wait_with_output().context(WaitFailedSnafu {
                command: "graph-easy",
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return CommandFailedSnafu {
                    command: "graph-easy".to_string(),
                    stderr: stderr.to_string(),
                }
                .fail();
            }

            let ascii_art = String::from_utf8_lossy(&output.stdout);

            if let Some(path) = output_path {
                std::fs::write(path, ascii_art.as_bytes()).context(super::IoSnafu)?;
            } else {
                // Print to stdout
                print!("{ascii_art}");
            }
        } else {
            // SVG, PNG, PDF
            let output_path =
                output_path.ok_or_else(|| OutputPathRequiredSnafu { format }.build())?;

            let format_flag = match format {
                DiagramFormat::Svg => "svg",
                DiagramFormat::Png => "png",
                DiagramFormat::Pdf => "pdf",
                DiagramFormat::Ascii => unreachable!(),
            };

            let mut cmd = Command::new(&self.dot_path)
                .arg(format!("-T{format_flag}"))
                .arg("-o")
                .arg(output_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context(SpawnFailedSnafu { command: "dot" })?;

            cmd.stdin
                .as_mut()
                .ok_or(StdinFailedSnafu.build())?
                .write_all(dot_source.as_bytes())
                .context(WriteStdinFailedSnafu)?;

            let output = cmd
                .wait_with_output()
                .context(WaitFailedSnafu { command: "dot" })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return CommandFailedSnafu {
                    command: "dot".to_string(),
                    stderr: stderr.to_string(),
                }
                .fail();
            }
        }

        Ok(())
    }

    fn is_available(&self) -> Result<bool> {
        Ok(Command::new(&self.dot_path)
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false))
    }

    fn version(&self) -> Result<String> {
        let output = Command::new(&self.dot_path)
            .arg("-V")
            .output()
            .context(ExecuteFailedSnafu { command: "dot -V" })?;

        if !output.status.success() {
            return CommandFailedSnafu {
                command: "dot -V".to_string(),
                stderr: "Command failed".to_string(),
            }
            .fail();
        }

        // Graphviz outputs version to stderr
        let version_str = String::from_utf8_lossy(&output.stderr);
        Ok(version_str.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

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
    fn test_graphviz_source_generation() {
        let workflow = create_test_workflow();
        let provider = GraphvizProvider::new();
        let source = provider.generate_source(&workflow, None).unwrap();

        assert!(source.contains("digraph"));
        assert!(source.contains("start"));
        assert!(source.contains("end"));
        assert!(source.contains("test-workflow"));
    }

    #[test]
    fn test_is_available() {
        let provider = GraphvizProvider::new();
        // This test just checks it doesn't panic
        let _ = provider.is_available();
    }
}
