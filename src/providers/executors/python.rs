use async_trait::async_trait;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{
    context::Context,
    executor::{Error, Executor, Result},
};

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
                let cache = self.function_cache.lock().unwrap();
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
            let mut cache = self.function_cache.lock().unwrap();
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

    /// Execute Python script using embedded interpreter with stdin, arguments (sys.argv), and environment variables
    async fn exec_script(
        &self,
        script: &str,
        stdin: Option<&str>,
        arguments: Option<&[String]>,
        environment: Option<&HashMap<String, String>>,
    ) -> Result<serde_json::Value> {
        // Use tokio::task::spawn_blocking since pyo3 operations need to block
        let script = script.to_string();
        let stdin = stdin.map(String::from);
        let arguments = arguments.map(<[String]>::to_vec);
        let environment = environment.cloned();

        tokio::task::spawn_blocking(move || {
            Python::with_gil(|py| {
                // Capture stdout and stderr
                let sys = PyModule::import_bound(py, "sys").map_err(|e| Error::Execution {
                    message: format!("Failed to import sys module: {e}"),
                })?;
                let io_module = PyModule::import_bound(py, "io").map_err(|e| Error::Execution {
                    message: format!("Failed to import io module: {e}"),
                })?;

                // Create StringIO objects for stdout and stderr
                let stdout_capture = io_module
                    .getattr("StringIO")
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to get StringIO: {e}"),
                    })?
                    .call0()
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to create StringIO: {e}"),
                    })?;
                let stderr_capture = io_module
                    .getattr("StringIO")
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to get StringIO: {e}"),
                    })?
                    .call0()
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to create StringIO: {e}"),
                    })?;

                // Save original stdout/stderr
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
                    sys.setattr("argv", argv_list).map_err(|e| Error::Execution {
                        message: format!("Failed to set sys.argv: {e}"),
                    })?;
                }

                // Set up environment variables in os.environ
                if let Some(env) = &environment {
                    let os_module = PyModule::import_bound(py, "os").map_err(|e| Error::Execution {
                        message: format!("Failed to import os module: {e}"),
                    })?;
                    let environ = os_module.getattr("environ").map_err(|e| Error::Execution {
                        message: format!("Failed to get os.environ: {e}"),
                    })?;
                    for (key, value) in env {
                        environ
                            .set_item(key, value)
                            .map_err(|e| Error::Execution {
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
                    sys.setattr("stdin", stdin_io).map_err(|e| Error::Execution {
                        message: format!("Failed to set sys.stdin: {e}"),
                    })?;
                }

                // Redirect stdout and stderr
                sys.setattr("stdout", &stdout_capture)
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to redirect stdout: {e}"),
                    })?;
                sys.setattr("stderr", &stderr_capture)
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to redirect stderr: {e}"),
                    })?;

                // Execute the script
                let result = py.run_bound(&script, None, None);

                // Restore original stdout/stderr/argv
                let _ = sys.setattr("stdout", orig_stdout);
                let _ = sys.setattr("stderr", orig_stderr);
                let _ = sys.setattr("argv", orig_argv);

                // Get captured output
                let stdout_str = stdout_capture
                    .call_method0("getvalue")
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to get stdout value: {e}"),
                    })?
                    .extract::<String>()
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to extract stdout string: {e}"),
                    })?;

                let stderr_str = stderr_capture
                    .call_method0("getvalue")
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to get stderr value: {e}"),
                    })?
                    .extract::<String>()
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to extract stderr string: {e}"),
                    })?;

                // Check if script execution failed
                if let Err(e) = result {
                    return Err(Error::Execution {
                        message: format!("Python script failed: {e}\nStderr: {stderr_str}"),
                    });
                }

                // Return stdout and stderr
                Ok(serde_json::json!({
                    "stdout": stdout_str,
                    "stderr": stderr_str,
                    "exitCode": 0
                }))
            })
        })
        .await
        .map_err(|e| Error::Execution {
            message: format!("Task join error: {e}"),
        })?
    }
}

#[async_trait]
impl Executor for PythonExecutor {
    async fn exec(
        &self,
        _task_name: &str,
        params: &serde_json::Value,
        _ctx: &Context,
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
            let environment = params.get("environment").and_then(|e| e.as_object()).map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<_, _>>()
            });

            self.exec_script(
                script,
                stdin,
                arguments.as_deref(),
                environment.as_ref(),
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
