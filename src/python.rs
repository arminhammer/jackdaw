//! Python bindings for jackdaw using PyO3
//!
//! This module provides Python wrappers for the core jackdaw functionality.
//! Enable with the "python" feature flag.

#![allow(unsafe_op_in_unsafe_fn)]

use crate::builder::DurableEngineBuilder;
use crate::durableengine::DurableEngine;
use crate::execution_handle::ExecutionHandle;
use crate::providers::container::DockerProvider;
use crate::session::Session;
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
/// Two core verbs:
///
/// - `preview(workflow_yaml)` — execute a step against the current context
///   and return the candidate result. Mutates nothing; call it repeatedly
///   while iterating on a step.
/// - `commit(workflow_yaml)` — accept a step: execute it, record it with
///   context snapshots, and advance the context. Committing an existing step
///   name rewinds to that step's prior input, replaces it in place, and
///   marks later steps stale; `replay()` heals them.
///
/// The session owns a dedicated Tokio runtime so every engine call lands on
/// the same runtime for the lifetime of the session.
#[pyclass(name = "Session")]
pub struct PySession {
    inner: Session,
    rt: tokio::runtime::Runtime,
}

impl PySession {
    fn parse_workflow(workflow_yaml: &str) -> PyResult<WorkflowDefinition> {
        serde_yaml::from_str(workflow_yaml)
            .map_err(|e| PyValueError::new_err(format!("Invalid workflow YAML: {e}")))
    }
}

#[pymethods]
impl PySession {
    /// Create a new session.
    ///
    /// `input` — initial context dict; defaults to an empty object when omitted.
    /// `input_schema` — JSON Schema document for the workflow's input; when
    ///   provided, the seed context is validated against it immediately and
    ///   the schema is embedded in the exported workflow.
    /// `output_schema` — JSON Schema document for the workflow's output,
    ///   embedded in the exported workflow.
    #[new]
    #[pyo3(signature = (input=None, input_schema=None, output_schema=None))]
    fn new(
        input: Option<Bound<'_, PyDict>>,
        input_schema: Option<Bound<'_, PyDict>>,
        output_schema: Option<Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let ctx = match input {
            Some(ref d) => python_dict_to_json(d)?,
            None => serde_json::Value::Object(Default::default()),
        };
        let input_schema = input_schema.as_ref().map(python_dict_to_json).transpose()?;
        let output_schema = output_schema
            .as_ref()
            .map(python_dict_to_json)
            .transpose()?;

        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {e}")))?;
        let session = Session::new(ctx, input_schema, output_schema)
            .map_err(|e| PyValueError::new_err(format!("Failed to create session: {e}")))?;
        Ok(Self { inner: session, rt })
    }

    /// Execute a step against the current context and return the candidate
    /// context dict. The session itself is not modified.
    #[pyo3(signature = (workflow_yaml, timeout_secs = 60.0))]
    fn preview(
        &self,
        py: Python<'_>,
        workflow_yaml: String,
        timeout_secs: f64,
    ) -> PyResult<PyObject> {
        let workflow = Self::parse_workflow(&workflow_yaml)?;
        let timeout = Duration::from_secs_f64(timeout_secs);

        let result = py
            .allow_threads(|| self.rt.block_on(self.inner.preview(workflow, timeout)))
            .map_err(|e| PyRuntimeError::new_err(format!("Preview failed: {e}")))?;

        json_to_python(py, &result)
    }

    /// Commit one step and return the updated context dict.
    ///
    /// `workflow_yaml` must contain exactly one named task. Re-committing an
    /// existing step name replaces that step (rewinding the context to the
    /// step's recorded input) and marks every later step stale.
    #[pyo3(signature = (workflow_yaml, timeout_secs = 60.0))]
    fn commit(
        &mut self,
        py: Python<'_>,
        workflow_yaml: String,
        timeout_secs: f64,
    ) -> PyResult<PyObject> {
        let workflow = Self::parse_workflow(&workflow_yaml)?;
        let timeout = Duration::from_secs_f64(timeout_secs);

        let inner = &mut self.inner;
        let rt = &self.rt;
        let result = py
            .allow_threads(|| rt.block_on(inner.commit(workflow, timeout)))
            .map_err(|e| PyRuntimeError::new_err(format!("Commit failed: {e}")))?;

        json_to_python(py, &result)
    }

    /// Re-execute every step from the first stale one onward and return the
    /// final context dict. No-op when nothing is stale. `timeout_secs`
    /// applies per step.
    #[pyo3(signature = (timeout_secs = 60.0))]
    fn replay(&mut self, py: Python<'_>, timeout_secs: f64) -> PyResult<PyObject> {
        let timeout = Duration::from_secs_f64(timeout_secs);

        let inner = &mut self.inner;
        let rt = &self.rt;
        let result = py
            .allow_threads(|| rt.block_on(inner.replay(timeout)))
            .map_err(|e| PyRuntimeError::new_err(format!("Replay failed: {e}")))?;

        json_to_python(py, &result)
    }

    /// Rewind the context to just before `name` ran, drop that step and
    /// everything after it, and return the restored context dict.
    fn rollback(&mut self, py: Python<'_>, name: &str) -> PyResult<PyObject> {
        let result = self
            .inner
            .rollback(name)
            .map_err(|e| PyRuntimeError::new_err(format!("Rollback failed: {e}")))?;
        json_to_python(py, &result)
    }

    /// Return the committed steps in order as a list of
    /// `{"name": str, "stale": bool}` dicts.
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyList>> {
        let list = pyo3::types::PyList::empty_bound(py);
        for step in self.inner.status() {
            let entry = PyDict::new_bound(py);
            entry.set_item("name", step.name)?;
            entry.set_item("stale", step.stale)?;
            list.append(entry)?;
        }
        Ok(list)
    }

    /// Return the current accumulated context as a Python dict.
    ///
    /// Note: this is a copy — in-place mutation is discarded. Use `update()`
    /// or assign to `ctx` to change the context.
    #[getter]
    fn ctx(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(py, self.inner.context())
    }

    /// Replace the current context with a Python dict.
    #[setter]
    fn set_ctx(&mut self, ctx: Bound<'_, PyDict>) -> PyResult<()> {
        let json = python_dict_to_json(&ctx)?;
        self.inner.set_context(json);
        Ok(())
    }

    /// Merge the top-level keys of `patch` into the current context
    /// (e.g. narrowing a list before committing the next step). Returns the
    /// updated context dict.
    fn update(&mut self, py: Python<'_>, patch: Bound<'_, PyDict>) -> PyResult<PyObject> {
        let json = python_dict_to_json(&patch)?;
        self.inner.update_context(json);
        json_to_python(py, self.inner.context())
    }

    /// Serialise all committed steps as a YAML workflow string.
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

/// Python-facing HTTP listener.
///
/// Wraps `HttpListener` with Python callable route handlers.  Each handler
/// receives the request payload as a Python dict and must return either:
///   - a plain dict  →  returned as `application/json`
///   - a dict matching the spec's HTTP Response schema
///     `{"statusCode": int, "headers": {"Content-Type": "..."}, "content": str}`
///     →  returned verbatim with the given status/headers/body.
///     For binary responses encode `content` as base-64; for text/* types pass
///     the string directly.
#[pyclass(name = "HttpListener")]
pub struct PyHttpListener {
    inner: Arc<crate::listeners::http::HttpListener>,
    // Multi-thread runtime that owns the spawned server task.
    // Kept alive so the background task keeps running after start() returns.
    rt: Arc<tokio::runtime::Runtime>,
}

#[pymethods]
impl PyHttpListener {
    #[new]
    fn new(bind_addr: String, routes: Bound<'_, PyDict>) -> PyResult<Self> {
        use crate::listeners::Error as ListenerError;
        use crate::listeners::http::HttpListener;
        use std::collections::HashMap;

        let mut route_handlers: HashMap<
            String,
            Arc<
                dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value>
                    + Send
                    + Sync,
            >,
        > = HashMap::new();

        for (key, val) in routes.iter() {
            let path: String = key.extract()?;
            let py_fn: PyObject = val.unbind();

            let handler: Arc<
                dyn Fn(serde_json::Value) -> crate::listeners::Result<serde_json::Value>
                    + Send
                    + Sync,
            > = Arc::new(move |value: serde_json::Value| {
                Python::with_gil(|py| {
                    let py_input =
                        json_to_python(py, &value).map_err(|e| ListenerError::Listener {
                            message: e.to_string(),
                        })?;
                    let result =
                        py_fn
                            .call1(py, (py_input,))
                            .map_err(|e| ListenerError::Listener {
                                message: e.to_string(),
                            })?;
                    let bound = result.into_bound(py);
                    if let Ok(dict) = bound.downcast::<PyDict>() {
                        python_dict_to_json(dict).map_err(|e| ListenerError::Listener {
                            message: e.to_string(),
                        })
                    } else {
                        Ok(serde_json::json!({}))
                    }
                })
            });

            route_handlers.insert(path, handler);
        }

        let listener = HttpListener::new_multi_route(bind_addr, route_handlers)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

        // Multi-thread runtime required so block_in_place (used by get_endpoint) works.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyRuntimeError::new_err(format!("runtime: {e}")))?;

        Ok(Self {
            inner: Arc::new(listener),
            rt: Arc::new(rt),
        })
    }

    /// Start the HTTP server in the background.  Returns immediately; the
    /// server keeps running as long as this object is alive.
    fn start(&self) -> PyResult<()> {
        use crate::listeners::Listener;
        let inner = self.inner.clone();
        self.rt
            .block_on(inner.start())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Gracefully stop the HTTP server.
    fn stop(&self) -> PyResult<()> {
        use crate::listeners::Listener;
        let inner = self.inner.clone();
        self.rt
            .block_on(inner.stop())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Return the bound endpoint string.
    #[getter]
    fn endpoint(&self) -> String {
        use crate::listeners::Listener;
        self.rt.block_on(async { self.inner.get_endpoint() })
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
/// Load registry credentials the way `docker build` does: from the local
/// container auth config's `auths` map. Sources checked in order:
/// `$REGISTRY_AUTH_FILE`, `$DOCKER_CONFIG/config.json`,
/// `~/.docker/config.json`, `$XDG_RUNTIME_DIR/containers/auth.json`
/// (podman login). Credential helpers (credsStore/credHelpers) are not
/// invoked — only static auth entries are forwarded.
///
/// Returns an empty map when nothing is found. The map is always passed to
/// bollard as `Some(..)` so the X-Registry-Config header is valid JSON
/// (`{}`); with `None` bollard sends an empty header value, which Podman's
/// docker-compat API rejects with "unexpected end of JSON input".
fn registry_credentials() -> std::collections::HashMap<String, bollard::auth::DockerCredentials> {
    use bollard::auth::DockerCredentials;
    use std::collections::HashMap;

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("REGISTRY_AUTH_FILE") {
        candidates.push(p.into());
    }
    if let Ok(p) = std::env::var("DOCKER_CONFIG") {
        candidates.push(std::path::Path::new(&p).join("config.json"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::Path::new(&home).join(".docker/config.json"));
    }
    if let Ok(p) = std::env::var("XDG_RUNTIME_DIR") {
        candidates.push(std::path::Path::new(&p).join("containers/auth.json"));
    }

    for path in candidates {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) else {
            continue;
        };
        let Some(auths) = json.get("auths").and_then(|a| a.as_object()) else {
            continue;
        };
        let mut creds = HashMap::new();
        for (registry, entry) in auths {
            let get = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(String::from);
            creds.insert(
                registry.clone(),
                DockerCredentials {
                    username: get("username"),
                    password: get("password"),
                    auth: get("auth"),
                    email: get("email"),
                    serveraddress: Some(registry.clone()),
                    identitytoken: get("identitytoken"),
                    registrytoken: get("registrytoken"),
                },
            );
        }
        if !creds.is_empty() {
            return creds;
        }
    }
    HashMap::new()
}

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
            let mut f = std::fs::File::create(&dockerfile_path)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to write Dockerfile: {e}")))?;
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

            for entry in walker
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            {
                let rel = entry
                    .path()
                    .strip_prefix(&ctx_path)
                    .map_err(|e| PyRuntimeError::new_err(format!("Path error: {e}")))?;
                archive
                    .append_path_with_name(entry.path(), rel)
                    .map_err(|e| {
                        PyRuntimeError::new_err(format!(
                            "Failed to add {} to tar: {e}",
                            rel.display()
                        ))
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

        let mut stream = docker.build_image(options, Some(registry_credentials()), Some(tar_bytes));
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(info) => {
                    if let Some(stream_line) = &info.stream {
                        print!("{stream_line}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                    if let Some(id) = info.aux.as_ref().and_then(|aux| aux.id.as_deref()) {
                        println!("Built image id: {id}");
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
        .import_bound("sys")
        .and_then(|sys| sys.getattr("executable"))
        .and_then(|exe| exe.extract::<String>())
    {
        crate::providers::executors::python::set_python_executable(exe);
    }

    m.add_class::<PyDurableEngine>()?;
    m.add_class::<PyDurableEngineBuilder>()?;
    m.add_class::<PyExecutionHandle>()?;
    m.add_class::<PySession>()?;
    m.add_class::<PyHttpListener>()?;
    m.add_function(wrap_pyfunction!(set_output_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(build_image, m)?)?;
    m.add_function(wrap_pyfunction!(stop_container, m)?)?;
    Ok(())
}
