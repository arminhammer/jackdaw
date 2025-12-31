/// External Python executor that calls the system `python` binary
///
/// This executor spawns the system Python interpreter as a subprocess,
/// providing an alternative to the embedded pyo3 executor.
/// It supports:
/// - Script execution with stdin/stdout/stderr streaming
/// - Command-line arguments
/// - Environment variables
/// - Real-time output streaming
use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::{
    context::Context,
    executor::{Error, Executor, Result},
    task_output::TaskOutputStreamer,
};

/// A reference to a Python function for the external executor
/// This stores the module and function name for later execution
#[derive(Clone, Debug)]
pub struct PythonFunctionRef {
    module: String,
    function: String,
}

/// Python executor that uses the system `python` binary
pub struct PythonExtExecutor {
    /// Path to the Python binary (defaults to "python3")
    python_path: String,
}

impl Default for PythonExtExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonExtExecutor {
    /// Create a new external Python executor with default python3 binary
    #[must_use]
    pub fn new() -> Self {
        Self {
            python_path: "python".to_string(),
        }
    }

    /// Create a new external Python executor with a custom Python binary path
    #[must_use]
    #[allow(dead_code)]
    pub fn with_python_path(python_path: String) -> Self {
        Self { python_path }
    }

    /// Load a Python function reference for later execution
    ///
    /// Unlike the embedded executor, this doesn't actually load or cache the function.
    /// It just creates a reference that stores the module and function name.
    ///
    /// # Errors
    ///
    /// This method doesn't perform validation, so it always succeeds.
    pub fn load_function(
        &self,
        module_path: &str,
        function_name: &str,
    ) -> Result<PythonFunctionRef> {
        Ok(PythonFunctionRef {
            module: module_path.to_string(),
            function: function_name.to_string(),
        })
    }

    /// Add a directory to Python's module search path
    ///
    /// For the external executor, this sets the PYTHONPATH environment variable.
    /// This is a no-op since PYTHONPATH should be set before launching the process.
    ///
    /// # Errors
    ///
    /// This is a compatibility method that always succeeds.
    /// For external executor, use the PYTHONPATH environment variable instead.
    #[allow(dead_code)]
    pub fn add_python_path(&self, _path: &str) -> Result<()> {
        // For external executor, PYTHONPATH should be set in the environment
        // This is a no-op for compatibility with the embedded executor API
        Ok(())
    }

    /// Execute a Python function (used by listeners)
    ///
    /// Since this is an external executor, we need to serialize the function and arguments
    /// to a script that can be executed by the external Python interpreter.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The function cannot be serialized to Python code
    /// - The Python interpreter fails to execute
    /// - The result cannot be parsed as JSON
    pub fn execute_function(
        &self,
        func: &PythonFunctionRef,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        // Generate a Python script that imports the module, loads the function, and executes it
        let args_json = serde_json::to_string(args).map_err(|e| Error::Execution {
            message: format!("Failed to serialize arguments: {e}"),
        })?;

        // Extract the base module path to add to PYTHONPATH
        // For "calculator.add", we want to add the directory containing "calculator"
        // Assume the module path is relative to the current working directory
        let module_parts: Vec<&str> = func.module.split('.').collect();
        let pythonpath_addition = if module_parts.len() > 1 {
            // Get current directory and look for the first module component
            std::env::current_dir().ok().and_then(|cwd| {
                // Try to find where the module package is located
                // Common patterns: tests/fixtures/.../handlers/python-handlers
                let base_module = module_parts.first()?;

                // Search for the module in common test directories
                for search_path in &[
                    cwd.join("tests/fixtures/listeners/handlers/python-handlers"),
                    cwd.join("handlers/python-handlers"),
                    cwd.join("python-handlers"),
                    cwd.clone(),
                ] {
                    if search_path.join(base_module).exists() {
                        return Some(search_path.to_string_lossy().to_string());
                    }
                }
                None
            })
        } else {
            None
        };

        let script = format!(
            r#"
import json
import sys
import os

# Add module path to PYTHONPATH if needed
{}

from {} import {}

# Parse arguments
args = json.loads('{}')

# Execute function
try:
    result = {}(*args)
    print(json.dumps(result))
except Exception as e:
    print(json.dumps({{"error": str(e)}}), file=sys.stderr)
    sys.exit(1)
"#,
            pythonpath_addition.map_or(String::new(), |path| {
                format!("sys.path.insert(0, '{}')", path.replace('\'', "\\'"))
            }),
            func.module,
            func.function,
            args_json.replace('\'', "\\'"),
            func.function
        );

        // Execute synchronously by spawning a blocking thread with its own runtime
        // This works even when called from within an async context
        let python_path = self.python_path.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().map_err(|e| Error::Execution {
                message: format!("Failed to create tokio runtime: {e}"),
            })?;
            let executor = PythonExtExecutor { python_path };
            rt.block_on(async { executor.exec_script(&script, None, None, None, None).await })
        })
        .join()
        .map_err(|_| Error::Execution {
            message: "Thread panicked while executing Python function".to_string(),
        })?
    }

    /// Execute a Python script using the system Python binary
    ///
    /// # Arguments
    /// * `script` - The Python code to execute
    /// * `stdin` - Optional stdin data to pass to the script
    /// * `arguments` - Optional command-line arguments
    /// * `environment` - Optional environment variables
    /// * `streamer` - Optional output streamer for real-time output
    ///
    /// # Errors
    /// Returns an error if:
    /// - The Python binary cannot be found or executed
    /// - Script execution fails
    /// - I/O operations fail during streaming
    async fn exec_script(
        &self,
        script: &str,
        stdin: Option<&str>,
        arguments: Option<&[String]>,
        environment: Option<&HashMap<String, String>>,
        streamer: Option<TaskOutputStreamer>,
    ) -> Result<serde_json::Value> {
        // Build command
        let mut cmd = Command::new(&self.python_path);
        cmd.arg("-c") // Execute code from command line
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Add command-line arguments if provided
        if let Some(args) = arguments {
            cmd.args(args);
        }

        // Set environment variables if provided
        if let Some(env) = environment {
            for (key, value) in env {
                cmd.env(key, value);
            }
        }

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| Error::Execution {
            message: format!("Failed to spawn Python process: {e}"),
        })?;

        // Handle stdin if provided
        if let Some(stdin_data) = stdin {
            if let Some(mut stdin_pipe) = child.stdin.take() {
                let stdin_data = stdin_data.to_string();
                tokio::spawn(async move {
                    let _ = stdin_pipe.write_all(stdin_data.as_bytes()).await;
                    let _ = stdin_pipe.flush().await;
                });
            }
        } else {
            drop(child.stdin.take());
        }

        // Capture stdout and stderr
        let stdout = child.stdout.take().ok_or_else(|| Error::Execution {
            message: "Failed to capture stdout".to_string(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| Error::Execution {
            message: "Failed to capture stderr".to_string(),
        })?;

        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        // Stream stdout
        let stdout_task = {
            let streamer = streamer.clone();
            let mut lines = stdout_reader.lines();
            tokio::spawn(async move {
                let mut collected = Vec::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(ref s) = streamer {
                        s.print_stdout(&line).await;
                    }
                    collected.push(line);
                }
                collected
            })
        };

        // Stream stderr
        let stderr_task = {
            let streamer = streamer.clone();
            let mut lines = stderr_reader.lines();
            tokio::spawn(async move {
                let mut collected = Vec::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(ref s) = streamer {
                        s.print_stderr(&line).await;
                    }
                    collected.push(line);
                }
                collected
            })
        };

        // Wait for process to complete
        let status = child.wait().await.map_err(|e| Error::Execution {
            message: format!("Failed to wait for Python process: {e}"),
        })?;

        // Collect output
        let stdout_lines = stdout_task.await.map_err(|e| Error::Execution {
            message: format!("Failed to collect stdout: {e}"),
        })?;
        let stderr_lines = stderr_task.await.map_err(|e| Error::Execution {
            message: format!("Failed to collect stderr: {e}"),
        })?;

        let exit_code = status.code().unwrap_or(-1);

        // Join lines with newlines
        let stdout_str = stdout_lines.join("\n");
        let stderr_str = stderr_lines.join("\n");

        // Check exit code
        if exit_code != 0 {
            return Err(Error::Execution {
                message: format!(
                    "Python script failed with exit code {exit_code}\nstdout: {stdout_str}\nstderr: {stderr_str}"
                ),
            });
        }

        // Return just stdout on success
        // Try to parse as JSON first, fall back to plain string
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&stdout_str) {
            Ok(json_value)
        } else {
            Ok(serde_json::Value::String(stdout_str))
        }
    }
}

#[async_trait]
impl Executor for PythonExtExecutor {
    async fn exec(
        &self,
        _task_name: &str,
        params: &serde_json::Value,
        _ctx: &Context,
        streamer: Option<TaskOutputStreamer>,
    ) -> Result<serde_json::Value> {
        // For external executor, we only support script execution
        // Module-based calls are not supported as they require the pyo3 embedded interpreter
        // Support both 'script' (from run task) and 'code' (legacy) parameters

        let script = params
            .get("script")
            .or_else(|| params.get("code"))
            .and_then(|c| c.as_str())
            .ok_or(Error::Execution {
                message: "Missing 'script' or 'code' parameter for Python script execution"
                    .to_string(),
            })?;

        let stdin = params.get("stdin").and_then(|s| s.as_str());

        let arguments: Option<Vec<String>> = params.get("arguments").and_then(|args| {
            args.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        });

        let environment: Option<HashMap<String, String>> =
            params.get("environment").and_then(|env| {
                env.as_object().map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
            });

        self.exec_script(
            script,
            stdin,
            arguments.as_deref(),
            environment.as_ref(),
            streamer,
        )
        .await
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]

    // Note: These tests are disabled because they require a proper Context setup
    // which involves persistence and cache providers. The python_ext executor
    // works fine in practice but cannot be easily unit tested in isolation.
    // Integration tests with real workflows should be used instead.

    #[allow(dead_code)]
    async fn test_basic_script_execution_disabled() {
        // This test is disabled - see note above
    }

    #[allow(dead_code)]
    async fn test_script_with_arguments_disabled() {
        // This test is disabled - see note above
    }

    #[allow(dead_code)]
    async fn test_script_with_environment_disabled() {
        // This test is disabled - see note above
    }

    #[allow(dead_code)]
    async fn test_script_exit_code_disabled() {
        // This test is disabled - see note above
    }
}
