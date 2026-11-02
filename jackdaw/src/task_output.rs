use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader as AsyncBufReader};
use tokio::process::Child;

/// Reports what a script task prints, line by line, as trace events (see [`crate::output::script_line`]).
pub struct TaskOutputStreamer {
    task_name: String,
    task_index: usize,
}

impl TaskOutputStreamer {
    /// Create a new output streamer for a task
    ///
    /// # Arguments
    /// * `task_name` - Name of the task
    /// * `task_index` - Index of the task (tells concurrent tasks apart)
    #[must_use]
    pub fn new(task_name: String, task_index: usize) -> Self {
        Self {
            task_name,
            task_index,
        }
    }

    /// Reports a line the task printed to stdout.
    pub async fn print_stdout(&self, line: &str) {
        crate::output::script_line(&self.task_name, self.task_index, "stdout", line);
    }

    /// Reports a line the task printed to stderr.
    pub async fn print_stderr(&self, line: &str) {
        crate::output::script_line(&self.task_name, self.task_index, "stderr", line);
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
    /// Returns (`stdout`, `stderr`, `exit_code`)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Reading from stdout or stderr fails
    /// - Waiting for the child process fails
    /// - A spawned task fails to complete
    pub async fn stream_process_output(
        &self,
        mut child: Child,
    ) -> std::io::Result<(String, String, i32)> {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("Failed to capture stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("Failed to capture stderr"))?;

        let stdout_handle = {
            let streamer = self.clone();
            crate::api::observe::spawn(async move { streamer.stream_reader(stdout, false).await })
        };

        let stderr_handle = {
            let streamer = self.clone();
            crate::api::observe::spawn(async move { streamer.stream_reader(stderr, true).await })
        };

        // Wait for process to complete
        let status = child.wait().await?;
        let exit_code = status.code().unwrap_or(0);

        // Collect output
        let stdout_lines = stdout_handle
            .await
            .map_err(|e| std::io::Error::other(format!("Stdout task failed: {e}")))??;

        let stderr_lines = stderr_handle
            .await
            .map_err(|e| std::io::Error::other(format!("Stderr task failed: {e}")))??;

        Ok((stdout_lines.join("\n"), stderr_lines.join("\n"), exit_code))
    }
}

// Implement Clone to allow sharing across tasks
impl Clone for TaskOutputStreamer {
    fn clone(&self) -> Self {
        Self {
            task_name: self.task_name.clone(),
            task_index: self.task_index,
        }
    }
}
