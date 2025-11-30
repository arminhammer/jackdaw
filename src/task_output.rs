use console::{Color, style};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader as AsyncBufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

/// ANSI color palette for task labels
static TASK_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

lazy_static::lazy_static! {
    /// Global output lock shared across all tasks to prevent output interleaving
    pub static ref OUTPUT_LOCK: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

/// Task output streamer that handles color-coded, labeled output
pub struct TaskOutputStreamer {
    task_name: String,
    color: Color,
}

impl TaskOutputStreamer {
    /// Create a new output streamer for a task
    ///
    /// # Arguments
    /// * `task_name` - Name of the task
    /// * `task_index` - Index of the task (used for color selection in concurrent scenarios)
    pub fn new(task_name: String, task_index: usize) -> Self {
        let color = TASK_COLORS[task_index % TASK_COLORS.len()];
        Self { task_name, color }
    }

    /// Format a line with task label and color
    fn format_line(&self, stream: &str, line: &str) -> String {
        let label = format!("[{}:{}]", self.task_name, stream);
        format!("{} {}", style(label).fg(self.color).bold(), line)
    }

    /// Print a single line to stdout with task label
    pub async fn print_stdout(&self, line: &str) {
        let formatted = self.format_line("stdout", line);
        let _lock = OUTPUT_LOCK.lock().await;
        println!("{}", formatted);
    }

    /// Print a single line to stderr with task label
    pub async fn print_stderr(&self, line: &str) {
        let formatted = self.format_line("stderr", line);
        let _lock = OUTPUT_LOCK.lock().await;
        eprintln!("{}", formatted);
    }

    /// Print multiple lines to stdout
    pub async fn print_stdout_lines(&self, lines: &[String]) {
        for line in lines {
            self.print_stdout(line).await;
        }
    }

    /// Print multiple lines to stderr
    pub async fn print_stderr_lines(&self, lines: &[String]) {
        for line in lines {
            self.print_stderr(line).await;
        }
    }

    /// Stream from a reader line by line
    async fn stream_reader<R: AsyncRead + Unpin>(
        &self,
        reader: R,
        is_stderr: bool,
    ) -> std::io::Result<Vec<String>> {
        let mut buf_reader = AsyncBufReader::new(reader).lines();
        let mut lines = Vec::new();

        while let Some(line) = buf_reader.next_line().await? {
            lines.push(line.clone());
            if is_stderr {
                self.print_stderr(&line).await;
            } else {
                self.print_stdout(&line).await;
            }
        }

        Ok(lines)
    }

    /// Stream stdout and stderr from a child process
    ///
    /// Returns (stdout, stderr, exit_code)
    pub async fn stream_process_output(
        &self,
        mut child: Child,
    ) -> std::io::Result<(String, String, i32)> {
        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let stderr = child.stderr.take().expect("Failed to capture stderr");

        let stdout_handle = {
            let streamer = self.clone();
            tokio::spawn(async move { streamer.stream_reader(stdout, false).await })
        };

        let stderr_handle = {
            let streamer = self.clone();
            tokio::spawn(async move { streamer.stream_reader(stderr, true).await })
        };

        // Wait for process to complete
        let status = child.wait().await?;
        let exit_code = status.code().unwrap_or(0);

        // Collect output
        let stdout_lines = stdout_handle.await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Stdout task failed: {}", e),
            )
        })??;

        let stderr_lines = stderr_handle.await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Stderr task failed: {}", e),
            )
        })??;

        Ok((stdout_lines.join("\n"), stderr_lines.join("\n"), exit_code))
    }
}

// Implement Clone to allow sharing across tasks
impl Clone for TaskOutputStreamer {
    fn clone(&self) -> Self {
        Self {
            task_name: self.task_name.clone(),
            color: self.color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_task_output_streamer() {
        let streamer = TaskOutputStreamer::new("test-task".to_string(), 0);

        // Test format_line
        let formatted = streamer.format_line("stdout", "Hello, world!");
        assert!(formatted.contains("test-task"));
        assert!(formatted.contains("stdout"));
        assert!(formatted.contains("Hello, world!"));
    }
}
