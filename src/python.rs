//! Python bindings for jackdaw using PyO3
//!
//! This module provides Python wrappers for the core jackdaw functionality.
//! Enable with the "python" feature flag.

#![allow(unsafe_op_in_unsafe_fn)]

use crate::builder::DurableEngineBuilder;
use crate::durableengine::DurableEngine;
use crate::execution_handle::ExecutionHandle;
use crate::session::Session;
use crate::providers::container::DockerProvider;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_asyncio_0_21 as pyo3_asyncio;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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
#[allow(unsafe_op_in_unsafe_fn)]
impl PyDurableEngineBuilder {
    #[new]
    fn new() -> Self {
        Self {
            inner: Some(DurableEngineBuilder::new()),
        }
    }

    unsafe fn build(&mut self) -> PyResult<PyDurableEngine> {
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
#[allow(unsafe_op_in_unsafe_fn)]
impl PyDurableEngine {
    #[staticmethod]
    fn builder() -> PyDurableEngineBuilder {
        PyDurableEngineBuilder::new()
    }

    unsafe fn execute<'py>(
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
#[allow(unsafe_op_in_unsafe_fn)]
impl PyExecutionHandle {
    unsafe fn wait_for_completion<'py>(
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

    unsafe fn instance_id(&self) -> String {
        self.instance_id.clone()
    }
}

/// Python-facing interactive pipeline session.
///
/// Each `run()` call parses the supplied single-step workflow YAML, executes it
/// through the jackdaw engine with the current context, and accumulates the
/// native `TaskDefinition` for later export.  The context is updated in place
/// after every step and exposed as a Python dict via the `ctx` property.
#[pyclass(name = "Session")]
pub struct PySession {
    inner: Session,
}

#[pymethods]
#[allow(unsafe_op_in_unsafe_fn)]
impl PySession {
    /// Create a new session.
    ///
    /// `input` — initial context dict; defaults to an empty object when omitted.
    #[new]
    #[pyo3(signature = (input=None))]
    unsafe fn new(input: Option<Bound<'_, PyDict>>) -> PyResult<Self> {
        let ctx = match input {
            Some(ref d) => python_dict_to_json(d)?,
            None => serde_json::Value::Object(Default::default()),
        };
        let session = Session::new(ctx)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create session: {e}")))?;
        Ok(Self { inner: session })
    }

    /// Execute one step and return the updated context dict.
    ///
    /// `workflow_yaml` — a complete single-step workflow YAML document produced
    ///   by `WorkflowBuilder.build().to_yaml()` on the Python side.
    /// `timeout_secs`  — per-step timeout in seconds (default 60).
    #[pyo3(signature = (workflow_yaml, timeout_secs = 60.0))]
    unsafe fn run(
        &mut self,
        py: Python<'_>,
        workflow_yaml: String,
        timeout_secs: f64,
    ) -> PyResult<PyObject> {
        use serverless_workflow_core::models::workflow::WorkflowDefinition;
        use std::time::Duration;

        let workflow: WorkflowDefinition =
            serde_yaml::from_str(&workflow_yaml).map_err(|e| {
                PyValueError::new_err(format!("Invalid workflow YAML: {e}"))
            })?;

        let timeout = Duration::from_secs_f64(timeout_secs);
        let result = self
            .inner
            .execute_step_sync(workflow, timeout)
            .map_err(|e| PyRuntimeError::new_err(format!("Step execution failed: {e}")))?;

        json_to_python(py, &result)
    }

    /// Return the current accumulated context as a Python dict.
    #[getter]
    unsafe fn ctx(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(py, self.inner.context())
    }

    /// Replace the current context with a Python dict.
    #[setter]
    unsafe fn set_ctx(&mut self, ctx: Bound<'_, PyDict>) -> PyResult<()> {
        let json = python_dict_to_json(&ctx)?;
        self.inner.set_context(json);
        Ok(())
    }

    /// Accumulate a step without executing it.
    ///
    /// Used in script mode: the step definition is registered so that
    /// `export_yaml()` includes it, but the engine is not invoked and the
    /// context is not changed.
    #[pyo3(signature = (workflow_yaml))]
    unsafe fn add(&mut self, workflow_yaml: String) -> PyResult<()> {
        use serverless_workflow_core::models::workflow::WorkflowDefinition;

        let workflow: WorkflowDefinition =
            serde_yaml::from_str(&workflow_yaml).map_err(|e| {
                PyValueError::new_err(format!("Invalid workflow YAML: {e}"))
            })?;

        self.inner.add_step(workflow);
        Ok(())
    }

    /// Serialise all accumulated steps as a YAML workflow string.
    #[pyo3(signature = (name, namespace = "default", version = "0.1.0"))]
    fn export_yaml(&self, name: String, namespace: &str, version: &str) -> PyResult<String> {
        self.inner
            .export_yaml(&name, namespace, version)
            .map_err(|e| PyRuntimeError::new_err(format!("Export failed: {e}")))
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

    if let Ok(val) = obj.extract::<f64>()
        && let Some(num) = serde_json::Number::from_f64(val)
    {
        return Ok(serde_json::Value::Number(num));
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

/// Enable or disable jackdaw's structured terminal output.
/// Defaults to True when using the Python SDK.
#[pyfunction]
fn set_output_enabled(enabled: bool) {
    crate::output::set_debug_mode(enabled);
}

/// Build a Docker image from a Dockerfile and optional build context directory.
///
/// `tag`         — image tag to apply, e.g. "sedona-spark4:local"
/// `dockerfile`  — Dockerfile content as a string (written into the context as "Dockerfile")
/// `context_dir` — path to the build context directory; defaults to a temp dir when empty.
///                 The Dockerfile string is always written into this dir (overwriting any
///                 existing Dockerfile), so you can point at a real project directory.
///
/// Build output is streamed to stderr in real time. Returns when the build completes
/// successfully or raises PyRuntimeError on failure.
///
/// Called from Python script steps generated by `build_image_task()` in the Python SDK.
/// Synchronous so callers need no asyncio setup.
#[pyfunction]
fn build_image(tag: String, dockerfile: String, context_dir: String) -> PyResult<()> {
    use bollard::image::BuildImageOptions;
    use bytes::Bytes;
    use futures::StreamExt;
    use std::io::Write as _;

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {e}")))?;

    rt.block_on(async move {
        // Resolve the context directory — use a temp dir if none given.
        let ctx_path: std::path::PathBuf = if context_dir.is_empty() {
            std::env::temp_dir().join(format!("jackdaw-build-{}", uuid::Uuid::new_v4()))
        } else {
            std::path::PathBuf::from(&context_dir)
        };
        std::fs::create_dir_all(&ctx_path).map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to create build context dir: {e}"))
        })?;

        // Write the Dockerfile into the context directory.
        let dockerfile_path = ctx_path.join("Dockerfile");
        {
            let mut f = std::fs::File::create(&dockerfile_path).map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to write Dockerfile: {e}"))
            })?;
            f.write_all(dockerfile.as_bytes()).map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to write Dockerfile contents: {e}"))
            })?;
        }

        // Tar the context directory into an in-memory buffer, honouring .dockerignore.
        let tar_bytes: Bytes = {
            let buf = std::io::Cursor::new(Vec::new());
            let mut archive = tar::Builder::new(buf);

            // WalkBuilder with .dockerignore support:
            //   - reads only the root-level .dockerignore (no nested gitignore files)
            //   - includes hidden files (dotfiles are valid in build contexts)
            //   - no git-specific exclusions
            let walker = ignore::WalkBuilder::new(&ctx_path)
                .add_custom_ignore_filename(".dockerignore")
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false)
                .hidden(false)
                .build();

            for entry in walker.filter_map(|e| e.ok()).filter(|e| e.file_type().map_or(false, |t| t.is_file())) {
                let rel = entry
                    .path()
                    .strip_prefix(&ctx_path)
                    .map_err(|e| PyRuntimeError::new_err(format!("Path error: {e}")))?;
                archive.append_path_with_name(entry.path(), rel).map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to add {} to tar: {e}", rel.display()))
                })?;
            }

            let inner = archive
                .into_inner()
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to finalise tar: {e}")))?;
            Bytes::from(inner.into_inner())
        };

        let docker = bollard::Docker::connect_with_local_defaults().map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to connect to Docker daemon: {e}"))
        })?;

        let options = BuildImageOptions {
            dockerfile: "Dockerfile",
            t: tag.as_str(),
            rm: true,
            ..Default::default()
        };

        let mut stream = docker.build_image(options, None, Some(tar_bytes));
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(info) => {
                    if let Some(stream_line) = &info.stream {
                        print!("{stream_line}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                    if let Some(aux) = &info.aux {
                        if let Some(id) = aux.id.as_deref() {
                            println!("Built image id: {id}");
                        }
                    }
                    if let Some(err) = &info.error {
                        return Err(PyRuntimeError::new_err(format!("Build error: {err}")));
                    }
                }
                Err(e) => {
                    return Err(PyRuntimeError::new_err(format!("Build stream error: {e}")));
                }
            }
        }

        Ok(())
    })
}

/// Stop a named container using jackdaw's bollard-based container runtime.
///
/// Connects to DOCKER_HOST (default: unix:///var/run/docker.sock) — the same
/// socket used by jackdaw's container task executor. Works with Docker, Podman
/// in socket mode, and any Docker-API-compatible runtime.
///
/// Called from Python script steps generated by `stop_container_task()` in the
/// Python SDK. Synchronous so callers need no asyncio setup.
#[pyfunction]
fn stop_container(container_name: String) -> PyResult<()> {
    use crate::container::ContainerProvider;
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {e}")))?;
    rt.block_on(async move {
        let provider = DockerProvider::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Docker provider error: {e}")))?;
        provider
            .stop_container(&container_name)
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("stop_container failed: {e}")))?;
        Ok(())
    })
}

#[pymodule]
fn _jackdaw(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Enable output by default in the Python SDK, matching CLI behaviour.
    // Users can suppress it with jackdaw.set_output_enabled(False).
    crate::output::set_debug_mode(true);

    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_indicatif::IndicatifLayer::new())
        .try_init();

    // Capture the host interpreter path so Python script steps inherit the same
    // virtualenv. Falls back to "python" if this fails (e.g. unusual embed setup).
    if let Ok(exe) = m
        .py()
        .import("sys")
        .and_then(|sys| sys.getattr("executable"))
        .and_then(|exe| exe.extract::<String>())
    {
        crate::providers::executors::python::set_python_executable(exe);
    }

    m.add_class::<PyDurableEngine>()?;
    m.add_class::<PyDurableEngineBuilder>()?;
    m.add_class::<PyExecutionHandle>()?;
    m.add_class::<PySession>()?;
    m.add_function(wrap_pyfunction!(set_output_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(build_image, m)?)?;
    m.add_function(wrap_pyfunction!(stop_container, m)?)?;
    Ok(())
}
