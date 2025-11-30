use async_trait::async_trait;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use std::collections::HashMap;
use std::fmt::Write;
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
            python_to_json(py, result.bind(py))
        })
    }

    /// Execute Python script from string with optional arguments injected as globals
    async fn exec_script(
        &self,
        script: &str,
        arguments: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        use std::io::Write;
        use tokio::process::Command;

        // If arguments are provided, inject them as global variables
        let full_script = if let Some(args) = arguments {
            let mut script_with_args = String::new();
            script_with_args.push_str("import json\n");

            // Inject each argument as a global variable
            if let Some(args_obj) = args.as_object() {
                for (key, value) in args_obj {
                    let value_json =
                        serde_json::to_string(value).map_err(|e| Error::Execution {
                            message: format!("Failed to serialize argument: {e}"),
                        })?;
                    writeln!(
                        script_with_args,
                        "{} = json.loads('{}')",
                        key,
                        value_json.replace('\'', "\\'")
                    )
                    .unwrap();
                }
            }

            script_with_args.push('\n');
            script_with_args.push_str(script);
            script_with_args
        } else {
            script.to_string()
        };

        let mut temp_file = tempfile::NamedTempFile::new().map_err(|e| Error::Execution {
            message: format!("Failed to create temp file: {e}"),
        })?;
        temp_file
            .write_all(full_script.as_bytes())
            .map_err(|e| Error::Execution {
                message: format!("Failed to write to temp file: {e}"),
            })?;
        temp_file.flush().map_err(|e| Error::Execution {
            message: format!("Failed to flush temp file: {e}"),
        })?;

        let output = Command::new("python3")
            .arg(temp_file.path())
            .output()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Failed to execute python3: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Execution {
                message: format!("Python script failed: {stderr}"),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(serde_json::from_str(&stdout).unwrap_or(serde_json::json!({ "result": stdout })))
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
                .map(|a| a.as_slice())
                .unwrap_or(&[]);

            // Load (or get cached) function
            let func = self.load_function(module_path, function_name)?;

            // Execute function
            self.execute_function(&func, args)
        } else if let Some(script) = params.get("script").and_then(|s| s.as_str()) {
            // Script mode: inline script with optional arguments
            let arguments = params.get("arguments");
            self.exec_script(script, arguments).await
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
fn python_to_json(py: Python, obj: &Bound<PyAny>) -> Result<serde_json::Value> {
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
            result.push(python_to_json(py, &item)?);
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
            result.insert(key_str, python_to_json(py, &value)?);
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
