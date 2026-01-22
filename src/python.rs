//! Python bindings for jackdaw using PyO3
//!
//! This module provides Python wrappers for the core jackdaw functionality.
//! Enable with the "python" feature flag.

use crate::builder::DurableEngineBuilder;
use crate::durableengine::DurableEngine;
use crate::execution_handle::ExecutionHandle;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_asyncio_0_21 as pyo3_asyncio;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::sync::Arc;
use std::time::Duration;

#[pyclass(name = "DurableEngine")]
pub struct PyDurableEngine {
    inner: Arc<DurableEngine>,
}

#[pyclass(name = "DurableEngineBuilder")]
pub struct PyDurableEngineBuilder {
    inner: Option<DurableEngineBuilder>,
}

#[pyclass(name = "ExecutionHandle")]
pub struct PyExecutionHandle {
    instance_id: String,
    handle: Option<ExecutionHandle>,
}

#[pymethods]
impl PyDurableEngineBuilder {
    #[new]
    fn new() -> Self {
        Self {
            inner: Some(DurableEngineBuilder::new()),
        }
    }

    fn build(&mut self) -> PyResult<PyDurableEngine> {
        let builder = self
            .inner
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Builder already consumed"))?;

        let engine = builder
            .build()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to build engine: {e}")))?;

        Ok(PyDurableEngine {
            inner: Arc::new(engine),
        })
    }
}

#[pymethods]
impl PyDurableEngine {
    #[staticmethod]
    fn builder() -> PyDurableEngineBuilder {
        PyDurableEngineBuilder::new()
    }

    fn execute<'py>(
        &self,
        py: Python<'py>,
        workflow_yaml: String,
        input: Bound<'py, PyDict>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let engine = Arc::clone(&self.inner);
        let input_json = python_dict_to_json(&input)?;

        pyo3_asyncio::tokio::future_into_py(py, async move {
            let workflow: WorkflowDefinition = serde_yaml::from_str(&workflow_yaml)
                .map_err(|e| PyValueError::new_err(format!("Invalid workflow YAML: {e}")))?;

            let handle = engine
                .execute(workflow, input_json)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Execution failed: {e}")))?;

            let instance_id = handle.instance_id().to_string();

            Ok(Python::with_gil(|py| {
                PyExecutionHandle {
                    instance_id,
                    handle: Some(handle),
                }
                .into_py(py)
            }))
        })
    }
}

#[pymethods]
impl PyExecutionHandle {
    fn wait_for_completion<'py>(
        &mut self,
        py: Python<'py>,
        timeout_secs: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let handle = self
            .handle
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Handle already consumed or invalid"))?;

        let timeout = Duration::from_secs_f64(timeout_secs);

        pyo3_asyncio::tokio::future_into_py(py, async move {
            let result = handle
                .wait_for_completion(timeout)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Execution failed: {e}")))?;

            Python::with_gil(|py| json_to_python(py, &result))
        })
    }

    fn instance_id(&self) -> String {
        self.instance_id.clone()
    }
}

fn python_dict_to_json(dict: &Bound<PyDict>) -> PyResult<serde_json::Value> {
    let items: Vec<(String, Bound<PyAny>)> = dict
        .iter()
        .map(|(k, v)| {
            let key: String = k
                .extract()
                .map_err(|_| PyValueError::new_err("Dictionary keys must be strings"))?;
            Ok((key, v))
        })
        .collect::<PyResult<Vec<_>>>()?;

    let mut map = serde_json::Map::new();
    for (key, value) in items {
        map.insert(key, pyany_to_json(&value)?);
    }

    Ok(serde_json::Value::Object(map))
}

fn pyany_to_json(obj: &Bound<PyAny>) -> PyResult<serde_json::Value> {
    if obj.is_none() {
        return Ok(serde_json::Value::Null);
    }

    if let Ok(val) = obj.extract::<bool>() {
        return Ok(serde_json::Value::Bool(val));
    }

    if let Ok(val) = obj.extract::<i64>() {
        return Ok(serde_json::Value::Number(val.into()));
    }

    if let Ok(val) = obj.extract::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(val) {
            return Ok(serde_json::Value::Number(num));
        }
    }

    if let Ok(val) = obj.extract::<String>() {
        return Ok(serde_json::Value::String(val));
    }

    if let Ok(list) = obj.downcast::<pyo3::types::PyList>() {
        let mut arr = Vec::new();
        for item in list.iter() {
            arr.push(pyany_to_json(&item)?);
        }
        return Ok(serde_json::Value::Array(arr));
    }

    if let Ok(dict) = obj.downcast::<pyo3::types::PyDict>() {
        return python_dict_to_json(dict);
    }

    Err(PyValueError::new_err(format!(
        "Unsupported Python type: {}",
        obj.get_type().name()?
    )))
}

fn json_to_python(py: Python, value: &serde_json::Value) -> PyResult<PyObject> {
    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok(b.into_py(py)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_py(py))
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_py(py))
            } else {
                Err(PyValueError::new_err("Invalid number"))
            }
        }
        serde_json::Value::String(s) => Ok(s.into_py(py)),
        serde_json::Value::Array(arr) => {
            let list = pyo3::types::PyList::new_bound(py, Vec::<PyObject>::new());
            for item in arr {
                list.append(json_to_python(py, item)?)?;
            }
            Ok(list.into())
        }
        serde_json::Value::Object(obj) => {
            let dict = PyDict::new_bound(py);
            for (key, val) in obj {
                dict.set_item(key, json_to_python(py, val)?)?;
            }
            Ok(dict.into())
        }
    }
}

#[pymodule]
fn jackdaw(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDurableEngine>()?;
    m.add_class::<PyDurableEngineBuilder>()?;
    m.add_class::<PyExecutionHandle>()?;
    Ok(())
}
