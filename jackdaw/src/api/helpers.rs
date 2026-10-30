//! The Serverless Workflow task types as `ctx` methods: run a command, a script, a container, or an HTTP request
//! from a Rust workflow.
//!
//! Each helper is a task the engine provides (`jackdaw.shell`, `jackdaw.script`, `jackdaw.container`,
//! `jackdaw.http`), so its result is journaled and not repeated on replay, and the step options of any task call
//! apply: `.retry(..)`, `.timeout(..)`, `.cached()`.
//!
//! ```no_run
//! # use jackdaw::prelude::*;
//! # use jackdaw::api::{Request, Shell};
//! # async fn run(ctx: &mut WorkflowContext<()>) -> WorkflowResult<()> {
//! let listing = ctx.run_shell(Shell::new("ls").args(["-l"])).await?;
//! let status = ctx.http(Request::get("https://example.com/health")).retry(Retry::exponential(5)).await?;
//! # Ok(()) }
//! ```
use super::error::{TaskError, error_types};
use super::task::{NativeTask, TaskDef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// What a command, script or container printed, and how it exited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct ProcessOutput {
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// The exit code (0: success; a non-zero code fails the step instead).
    pub exit_code: i64,
}

/// A command for [`ctx.run_shell`](crate::api::WorkflowContext::run_shell). It runs directly, not through a shell:
/// arguments are passed as they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct Shell {
    /// The program.
    pub command: String,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// The working directory (the engine's when unset).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Written to the command's standard input.
    #[serde(default)]
    pub stdin: Option<String>,
}

impl Shell {
    /// Runs `command`.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            stdin: None,
        }
    }

    /// Adds arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    /// Sets the working directory.
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<String>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Sets standard input.
    #[must_use]
    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin = Some(input.into());
        self
    }
}

/// A script for [`ctx.run_script`](crate::api::WorkflowContext::run_script), run with the language's interpreter
/// (`python3`, `node`) on the engine's machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct Script {
    /// `python` or `javascript`.
    pub language: String,
    /// The code.
    pub code: String,
    /// Arguments the script sees (`sys.argv[1:]`, `process.argv.slice(2)`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Script {
    /// Python code, run with `python3`.
    #[must_use]
    pub fn python(code: impl Into<String>) -> Self {
        Self::new("python", code)
    }

    /// JavaScript code, run with `node`.
    #[must_use]
    pub fn javascript(code: impl Into<String>) -> Self {
        Self::new("javascript", code)
    }

    fn new(language: &str, code: impl Into<String>) -> Self {
        Self {
            language: language.to_string(),
            code: code.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    /// Adds arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    fn shell(&self) -> Result<Shell, TaskError> {
        let (interpreter, flag) = match self.language.as_str() {
            "python" => ("python3", "-c"),
            "javascript" | "js" => ("node", "-e"),
            other => {
                return Err(TaskError::fault(error_types::CONFIGURATION).detail(format!(
                    "scripts in `{other}` are not supported (use python or javascript)"
                )));
            }
        };
        let mut shell = Shell::new(interpreter).args([flag, self.code.as_str()]);
        if self.language != "python" {
            // `node -e code a b` passes `a b` as script arguments only after `--`.
            shell = shell.args(["--"]);
        }
        shell.args.extend(self.args.iter().cloned());
        shell.env.clone_from(&self.env);
        Ok(shell)
    }
}

/// A container for [`ctx.run_container`](crate::api::WorkflowContext::run_container), run with Docker and removed
/// when it exits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct Container {
    /// The image, e.g. `alpine:3`.
    pub image: String,
    /// The command (the image's default when empty).
    #[serde(default)]
    pub command: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Written to the container's standard input.
    #[serde(default)]
    pub stdin: Option<String>,
}

impl Container {
    /// Runs `image`.
    #[must_use]
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            command: Vec::new(),
            env: BTreeMap::new(),
            stdin: None,
        }
    }

    /// Sets the command.
    #[must_use]
    pub fn command<I, S>(mut self, command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.command = command.into_iter().map(Into::into).collect();
        self
    }

    /// Sets an environment variable.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    /// Sets standard input.
    #[must_use]
    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin = Some(input.into());
        self
    }
}

/// An HTTP request for [`ctx.http`](crate::api::WorkflowContext::http).
///
/// Statuses 408, 429 and 5xx (and connection errors) fail the step with a retryable fault, so a step `.retry(..)`
/// policy retries them; other 4xx statuses fail it for good. Any other response is returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct Request {
    /// `GET`, `POST`, ...
    pub method: String,
    /// The URL.
    pub url: String,
    /// Request headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// A JSON body.
    #[serde(default)]
    pub body: Option<Value>,
}

impl Request {
    /// A request with `method`.
    #[must_use]
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            method: method.into().to_uppercase(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: None,
        }
    }

    /// A `GET` request.
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self::new("GET", url)
    }

    /// A `POST` request with a JSON body.
    #[must_use]
    pub fn post(url: impl Into<String>, body: impl Serialize) -> Self {
        Self::new("POST", url).json(body)
    }

    /// A `PUT` request with a JSON body.
    #[must_use]
    pub fn put(url: impl Into<String>, body: impl Serialize) -> Self {
        Self::new("PUT", url).json(body)
    }

    /// A `DELETE` request.
    #[must_use]
    pub fn delete(url: impl Into<String>) -> Self {
        Self::new("DELETE", url)
    }

    /// Sets a header.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Sets a JSON body (`null` if `body` does not serialize).
    #[must_use]
    pub fn json(mut self, body: impl Serialize) -> Self {
        self.body = Some(serde_json::to_value(body).unwrap_or_default());
        self
    }
}

/// The response to [`ctx.http`](crate::api::WorkflowContext::http).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[non_exhaustive]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Response headers (lower-case names).
    pub headers: BTreeMap<String, String>,
    /// The body: parsed JSON when it is JSON, a string otherwise, `null` when empty.
    pub body: Value,
}

macro_rules! helper_marker {
    ($marker:ident, $name:literal, $input:ty, $output:ty) => {
        #[derive(Debug, Clone, Copy)]
        pub(crate) struct $marker;

        impl TaskDef for $marker {
            type In = $input;
            type Out = $output;

            fn task_name(&self) -> &'static str {
                $name
            }
        }
    };
}

helper_marker!(ShellTask, "jackdaw.shell", Shell, ProcessOutput);
helper_marker!(ScriptTask, "jackdaw.script", Script, ProcessOutput);
helper_marker!(ContainerTask, "jackdaw.container", Container, ProcessOutput);
helper_marker!(HttpTask, "jackdaw.http", Request, Response);

/// The helper tasks every engine provides.
pub(crate) fn builtin_tasks() -> Vec<NativeTask> {
    vec![
        NativeTask::new("jackdaw.shell", false, |_ctx, shell: Shell| {
            run_shell(shell)
        }),
        NativeTask::new("jackdaw.script", false, |_ctx, script: Script| async move {
            run_shell(script.shell()?).await
        }),
        NativeTask::new("jackdaw.container", false, |_ctx, container: Container| {
            run_container(container)
        }),
        NativeTask::new("jackdaw.http", false, |_ctx, request: Request| {
            send(request)
        }),
    ]
}

fn runtime_fault(title: impl std::fmt::Display, detail: impl std::fmt::Display) -> TaskError {
    TaskError::fault(error_types::RUNTIME)
        .title(title)
        .detail(detail)
}

/// A non-zero exit fails the step: its title names the exit code and its detail is what was printed to stderr.
fn exited(output: ProcessOutput) -> Result<ProcessOutput, TaskError> {
    if output.exit_code == 0 {
        Ok(output)
    } else {
        let detail = if output.stderr.trim().is_empty() {
            output.stdout
        } else {
            output.stderr
        };
        Err(runtime_fault(
            format!("exit code {}", output.exit_code),
            detail.trim(),
        ))
    }
}

async fn run_shell(shell: Shell) -> Result<ProcessOutput, TaskError> {
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new(&shell.command);
    command
        .args(&shell.args)
        .envs(&shell.env)
        .stdin(if shell.stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = &shell.cwd {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .map_err(|e| runtime_fault(format!("`{}` could not be started", shell.command), e))?;
    if let (Some(input), Some(mut stdin)) = (shell.stdin, child.stdin.take()) {
        stdin
            .write_all(input.as_bytes())
            .await
            .map_err(|e| runtime_fault("standard input could not be written", e))?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| runtime_fault(format!("`{}` failed", shell.command), e))?;
    exited(ProcessOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        // A process killed by a signal has no code.
        exit_code: output.status.code().map_or(-1, i64::from),
    })
}

async fn run_container(container: Container) -> Result<ProcessOutput, TaskError> {
    use crate::container::{ContainerConfig, ContainerProvider};
    let provider = crate::providers::container::DockerProvider::new()
        .map_err(|e| runtime_fault("Docker is not available", e))?;
    let result = provider
        .execute(ContainerConfig {
            image: container.image.clone(),
            command: container.command,
            stdin: container.stdin,
            environment: (!container.env.is_empty()).then(|| container.env.into_iter().collect()),
            working_dir: None,
            volumes: None,
            ports: None,
        })
        .await
        .map_err(|e| runtime_fault(format!("container `{}` could not run", container.image), e))?;
    exited(ProcessOutput {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code,
    })
}

fn retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

async fn send(request: Request) -> Result<Response, TaskError> {
    let method = reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| {
        TaskError::fault(error_types::CONFIGURATION)
            .detail(format!("`{}` is not an HTTP method: {e}", request.method))
    })?;
    let mut builder = reqwest::Client::new().request(method, &request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = &request.body {
        builder = builder.json(body);
    }
    let response = builder.send().await.map_err(|e| {
        TaskError::fault(error_types::COMMUNICATION)
            .detail(format!("{} {} failed: {e}", request.method, request.url))
            .retry()
    })?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
        })
        .collect();
    let text = response.text().await.map_err(|e| {
        TaskError::fault(error_types::COMMUNICATION)
            .detail(format!(
                "the response to {} {} could not be read: {e}",
                request.method, request.url
            ))
            .retry()
    })?;
    let body = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    if status >= 400 {
        let fault = TaskError::fault(error_types::COMMUNICATION)
            .status(status)
            .title(format!(
                "{} {} returned {status}",
                request.method, request.url
            ))
            .detail(
                body.as_str()
                    .map_or_else(|| body.to_string(), str::to_string),
            );
        return Err(if retryable_status(status) {
            fault.retry()
        } else {
            fault
        });
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}
