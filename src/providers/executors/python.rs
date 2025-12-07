// Allow unsafe operations in PyO3-generated code
#![allow(unsafe_op_in_unsafe_fn)]

use async_trait::async_trait;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};

use crate::{
    context::Context,
    executor::{Error, Executor, Result},
};

/// A Python file-like object that sends output line-by-line through a channel
/// This enables real-time streaming from Python (sync) to Rust async tasks
#[pyclass]
struct StreamWriter {
    sender: mpsc::Sender<String>,
    buffer: Arc<Mutex<String>>,
}

#[pymethods]
impl StreamWriter {
    fn write(&self, s: &str) {
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer.push_str(s);

        // Send complete lines immediately through the channel
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer.drain(..=newline_pos);
            // Ignore send errors (receiver might be dropped if cancelled)
            let _ = self.sender.send(line);
        }
    }

    fn flush(&self) {
        // Flush any remaining buffered content as a final line
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        if !buffer.is_empty() {
            let _ = self.sender.send(buffer.clone());
            buffer.clear();
        }
    }

    fn isatty(&self) -> bool {
        false
    }
}

/// Python executor using pyo3 free-threaded mode for in-process execution with hot module caching
pub struct PythonExecutor {
    /// Cache of loaded Python functions
    /// Key: `"module_name.function_name"`
    /// Value: Cached Python function object (unbound, can be used across threads)
    function_cache: Arc<Mutex<HashMap<String, Py<PyAny>>>>,
}

impl Default for PythonExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonExecutor {
    /// Create a new Python executor with empty cache
    #[must_use]
    pub fn new() -> Self {
        // Initialize Python interpreter if not already initialized
        // Since we're not using auto-initialize, we need to manually prepare the interpreter
        pyo3::prepare_freethreaded_python();

        Self {
            function_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Load a Python function and cache it
    /// Returns a cached reference to the function
    /// Uses free-threaded mode without GIL
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The Python module cannot be imported
    /// - The function is not found in the module
    /// - The function is not callable
    ///
    /// # Panics
    ///
    /// Panics if the function cache mutex is poisoned
    pub fn load_function(&self, module_path: &str, function_name: &str) -> Result<Py<PyAny>> {
        let cache_key = format!("{module_path}.{function_name}");

        // Check cache first - need to acquire GIL for clone_ref
        Python::with_gil(|py| {
            {
                let cache = self.function_cache.lock().map_err(|e| Error::Execution {
                    message: format!("Failed to acquire function cache lock: {e}"),
                })?;
                if let Some(func) = cache.get(&cache_key) {
                    return Ok(func.clone_ref(py));
                }
            }

            // Load the module and function
            let module = PyModule::import_bound(py, module_path).map_err(|e| Error::Execution {
                message: format!("Failed to import module '{module_path}': {e}"),
            })?;
            // Get the function from the module
            let function = module
                .getattr(function_name)
                .map_err(|e| Error::Execution {
                    message: format!(
                        "Function '{function_name}' not found in module '{module_path}': {e}"
                    ),
                })?;

            // Check if it's callable
            if !function.is_callable() {
                return Err(Error::Execution {
                    message: format!("'{function_name}' in module '{module_path}' is not callable"),
                });
            }

            // Cache and return the unbound function object
            let func_obj = function.unbind();
            let mut cache = self.function_cache.lock().map_err(|e| Error::Execution {
                message: format!("Failed to acquire function cache lock: {e}"),
            })?;
            cache.insert(cache_key, func_obj.clone_ref(py));

            Ok(func_obj)
        })
    }

    /// Add a directory to Python's sys.path for module resolution
    /// This is useful for adding custom module paths at runtime
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Importing the sys module fails
    /// - Getting sys.path attribute fails
    /// - Inserting the path into sys.path fails
    #[allow(dead_code)]
    pub fn add_python_path(&self, path: &str) -> Result<()> {
        Python::with_gil(|py| {
            let sys = PyModule::import_bound(py, "sys").map_err(|e| Error::Execution {
                message: format!("Failed to import sys module: {e}"),
            })?;
            let sys_path: Bound<PyAny> = sys.getattr("path").map_err(|e| Error::Execution {
                message: format!("Failed to get sys.path: {e}"),
            })?;
            sys_path
                .call_method1("insert", (0, path))
                .map_err(|e| Error::Execution {
                    message: format!("Failed to add path to sys.path: {e}"),
                })?;
            Ok(())
        })
    }

    /// Execute a Python function with given arguments
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Converting JSON arguments to Python objects fails
    /// - The Python function call fails
    /// - Converting the Python result back to JSON fails
    pub fn execute_function(
        &self,
        func: &Py<PyAny>,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        Python::with_gil(|py| {
            // Convert JSON arguments to Python objects
            let py_args: Vec<PyObject> = args
                .iter()
                .map(|arg| json_to_python(py, arg))
                .collect::<Result<Vec<_>>>()?;

            // Call the function
            let result = func
                .call1(py, pyo3::types::PyTuple::new_bound(py, &py_args))
                .map_err(|e| Error::Execution {
                    message: format!("Python function call failed: {e}"),
                })?;

            // Convert result back to JSON
            python_to_json(result.bind(py))
        })
    }

    /// Execute Python script using embedded interpreter with TRUE real-time streaming
    ///
    /// Uses separate thread for Python execution and async task for streaming output
    /// This enables real-time output even while Python is executing (no GIL blocking streaming)
    async fn exec_script(
        &self,
        script: &str,
        stdin: Option<&str>,
        arguments: Option<&[String]>,
        environment: Option<&HashMap<String, String>>,
        streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> Result<serde_json::Value> {
        let script = script.to_string();
        let stdin = stdin.map(String::from);
        let arguments = arguments.map(<[String]>::to_vec);
        let environment = environment.cloned();

        // Create channels for streaming stdout and stderr from Python thread to async task
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>();
        let (stderr_tx, stderr_rx) = mpsc::channel::<String>();

        // Spawn OS thread to execute Python (not spawn_blocking, to avoid blocking executor)
        let python_handle = std::thread::spawn(move || -> Result<i32> {
            Python::with_gil(|py| {
                // Create StreamWriter instances
                let stdout_writer = Py::new(
                    py,
                    StreamWriter {
                        sender: stdout_tx,
                        buffer: Arc::new(Mutex::new(String::new())),
                    },
                )
                .map_err(|e| Error::Execution {
                    message: format!("Failed to create stdout writer: {e}"),
                })?;

                let stderr_writer = Py::new(
                    py,
                    StreamWriter {
                        sender: stderr_tx,
                        buffer: Arc::new(Mutex::new(String::new())),
                    },
                )
                .map_err(|e| Error::Execution {
                    message: format!("Failed to create stderr writer: {e}"),
                })?;

                // Get sys and io modules
                let sys = PyModule::import_bound(py, "sys").map_err(|e| Error::Execution {
                    message: format!("Failed to import sys module: {e}"),
                })?;
                let io_module = PyModule::import_bound(py, "io").map_err(|e| Error::Execution {
                    message: format!("Failed to import io module: {e}"),
                })?;

                // Save original stdout/stderr/argv
                let orig_stdout = sys.getattr("stdout").map_err(|e| Error::Execution {
                    message: format!("Failed to get original stdout: {e}"),
                })?;
                let orig_stderr = sys.getattr("stderr").map_err(|e| Error::Execution {
                    message: format!("Failed to get original stderr: {e}"),
                })?;
                let orig_argv = sys.getattr("argv").map_err(|e| Error::Execution {
                    message: format!("Failed to get original argv: {e}"),
                })?;

                // Set up sys.argv
                if let Some(args) = &arguments {
                    let argv_list = pyo3::types::PyList::empty_bound(py);
                    argv_list.append("").map_err(|e| Error::Execution {
                        message: format!("Failed to append to argv: {e}"),
                    })?; // Script name placeholder
                    for arg in args {
                        argv_list.append(arg).map_err(|e| Error::Execution {
                            message: format!("Failed to append argument to argv: {e}"),
                        })?;
                    }
                    sys.setattr("argv", argv_list)
                        .map_err(|e| Error::Execution {
                            message: format!("Failed to set sys.argv: {e}"),
                        })?;
                }

                // Set up environment variables in os.environ
                if let Some(env) = &environment {
                    let os_module =
                        PyModule::import_bound(py, "os").map_err(|e| Error::Execution {
                            message: format!("Failed to import os module: {e}"),
                        })?;
                    let environ = os_module.getattr("environ").map_err(|e| Error::Execution {
                        message: format!("Failed to get os.environ: {e}"),
                    })?;
                    for (key, value) in env {
                        environ.set_item(key, value).map_err(|e| Error::Execution {
                            message: format!("Failed to set environment variable: {e}"),
                        })?;
                    }
                }

                // Set up stdin if provided
                if let Some(stdin_data) = &stdin {
                    let stdin_io = io_module
                        .getattr("StringIO")
                        .map_err(|e| Error::Execution {
                            message: format!("Failed to get StringIO for stdin: {e}"),
                        })?
                        .call1((stdin_data,))
                        .map_err(|e| Error::Execution {
                            message: format!("Failed to create StringIO for stdin: {e}"),
                        })?;
                    sys.setattr("stdin", stdin_io)
                        .map_err(|e| Error::Execution {
                            message: format!("Failed to set sys.stdin: {e}"),
                        })?;
                }

                // Redirect stdout and stderr to our StreamWriters
                sys.setattr("stdout", &stdout_writer)
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to redirect stdout: {e}"),
                    })?;
                sys.setattr("stderr", &stderr_writer)
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to redirect stderr: {e}"),
                    })?;

                // Execute the script - output will stream to channels in real-time!
                let result = py.run_bound(&script, None, None);

                // Flush any remaining buffered output
                let _ = stdout_writer.call_method0(py, "flush");
                let _ = stderr_writer.call_method0(py, "flush");

                // Restore original stdout/stderr/argv
                let _ = sys.setattr("stdout", orig_stdout);
                let _ = sys.setattr("stderr", orig_stderr);
                let _ = sys.setattr("argv", orig_argv);

                // Check if script execution failed
                if let Err(e) = result {
                    return Err(Error::Execution {
                        message: format!("Python script failed: {e}"),
                    });
                }

                // Return exit code (stdout/stderr already streamed)
                Ok(0)
            })
        });

        // Spawn async task to receive and stream output in real-time
        let stdout_lines = Arc::new(Mutex::new(Vec::new()));
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));
        let stdout_lines_clone = Arc::clone(&stdout_lines);
        let stderr_lines_clone = Arc::clone(&stderr_lines);

        let streaming_task = tokio::spawn(async move {
            // Receive from both channels and stream output
            let stdout_task = {
                let stdout_lines = Arc::clone(&stdout_lines_clone);
                let streamer = streamer.clone();
                tokio::task::spawn_blocking(move || {
                    while let Ok(line) = stdout_rx.recv() {
                        if let Some(ref s) = streamer {
                            // Stream the line in real-time
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(async {
                                    s.print_stdout(&line).await;
                                });
                            });
                        }
                        stdout_lines
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(line);
                    }
                })
            };

            let stderr_task = {
                let stderr_lines = stderr_lines_clone;
                let streamer = streamer.clone();
                tokio::task::spawn_blocking(move || {
                    while let Ok(line) = stderr_rx.recv() {
                        if let Some(ref s) = streamer {
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(async {
                                    s.print_stderr(&line).await;
                                });
                            });
                        }
                        stderr_lines
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(line);
                    }
                })
            };

            let _ = tokio::join!(stdout_task, stderr_task);
        });

        // Wait for Python thread to complete
        let exit_code = python_handle.join().map_err(|_| Error::Execution {
            message: "Python thread panicked".to_string(),
        })??;

        // Wait for streaming to complete
        streaming_task.await.map_err(|e| Error::Execution {
            message: format!("Streaming task failed: {e}"),
        })?;

        // Collect final output
        let stdout_str = stdout_lines
            .lock()
            .map_err(|e| Error::Execution {
                message: format!("Mutex poisoned: {e}"),
            })?
            .join("\n");
        let stderr_str = stderr_lines
            .lock()
            .map_err(|e| Error::Execution {
                message: format!("Mutex poisoned: {e}"),
            })?
            .join("\n");

        Ok(serde_json::json!({
            "stdout": stdout_str,
            "stderr": stderr_str,
            "exitCode": exit_code
        }))
    }
}

#[async_trait]
impl Executor for PythonExecutor {
    async fn exec(
        &self,
        _task_name: &str,
        params: &serde_json::Value,
        _ctx: &Context,
        streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> Result<serde_json::Value> {
        // Check if this is a module-based call (new style) or script-based (legacy)
        if let Some(module_path) = params.get("module").and_then(|m| m.as_str()) {
            // New style: module + function + arguments
            let function_name =
                params
                    .get("function")
                    .and_then(|f| f.as_str())
                    .ok_or(Error::Execution {
                        message: "Missing 'function' parameter".to_string(),
                    })?;

            let args = params
                .get("arguments")
                .and_then(|a| a.as_array())
                .map_or(&[] as &[serde_json::Value], Vec::as_slice);

            // Load (or get cached) function
            let func = self.load_function(module_path, function_name)?;

            // Execute function
            self.execute_function(&func, args)
        } else if let Some(script) = params.get("script").and_then(|s| s.as_str()) {
            // Script mode: inline script with optional stdin, arguments, and environment
            let stdin = params.get("stdin").and_then(|s| s.as_str());
            let arguments = params
                .get("arguments")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                });
            let environment = params
                .get("environment")
                .and_then(|e| e.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<HashMap<_, _>>()
                });

            self.exec_script(
                script,
                stdin,
                arguments.as_deref(),
                environment.as_ref(),
                streamer,
            )
            .await
        } else {
            Err(Error::Execution {
                message:
                    "Python executor requires either 'module' + 'function' or 'script' parameter"
                        .to_string(),
            })
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Convert JSON value to Python object
fn json_to_python(py: Python, value: &serde_json::Value) -> Result<PyObject> {
    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok(b.into_py(py)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_py(py))
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_py(py))
            } else {
                Err(Error::Execution {
                    message: "Unsupported number type".to_string(),
                })
            }
        }
        serde_json::Value::String(s) => Ok(s.into_py(py)),
        serde_json::Value::Array(arr) => {
            let py_list: Result<Vec<PyObject>> =
                arr.iter().map(|v| json_to_python(py, v)).collect();
            Ok(py_list?.into_py(py))
        }
        serde_json::Value::Object(obj) => {
            let dict = PyDict::new_bound(py);
            for (key, value) in obj {
                let py_value = json_to_python(py, value)?;
                dict.set_item(key, py_value).map_err(|e| Error::Execution {
                    message: format!("Failed to set dict item: {e}"),
                })?;
            }
            Ok(dict.into_py(py))
        }
    }
}

/// Convert Python object to JSON value
fn python_to_json(obj: &Bound<PyAny>) -> Result<serde_json::Value> {
    // Check for None
    if obj.is_none() {
        return Ok(serde_json::Value::Null);
    }

    // Try bool (must check before int, as bool is subclass of int in Python)
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(serde_json::Value::Bool(b));
    }

    // Try int
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(serde_json::json!(i));
    }

    // Try float
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(serde_json::json!(f));
    }

    // Try string
    if let Ok(s) = obj.extract::<String>() {
        return Ok(serde_json::Value::String(s));
    }

    // Try list
    if let Ok(list) = obj.downcast::<pyo3::types::PyList>() {
        let mut result = Vec::new();
        for item in list.iter() {
            result.push(python_to_json(&item)?);
        }
        return Ok(serde_json::Value::Array(result));
    }

    // Try dict
    if let Ok(dict) = obj.downcast::<pyo3::types::PyDict>() {
        let mut result = serde_json::Map::new();
        for (key, value) in dict.iter() {
            let key_str = key.extract::<String>().map_err(|_| Error::Execution {
                message: "Dict keys must be strings".to_string(),
            })?;
            result.insert(key_str, python_to_json(&value)?);
        }
        return Ok(serde_json::Value::Object(result));
    }

    Err(Error::Execution {
        message: format!(
            "Unsupported Python type: {}",
            obj.get_type().name().map_err(|e| Error::Execution {
                message: format!("Failed to get type name: {e}")
            })?
        ),
    })
}
