//! `call: mcp`: one request to a [Model Context Protocol](https://modelcontextprotocol.io/) server.
//!
//! Each call connects with the official MCP SDK (`rmcp`), which sends `initialize` and the `initialized`
//! notification before anything else; then the one request the task names goes out, and the call disconnects. A task
//! is one call, so nothing is kept between tasks.
//!
//! - `stdio`: the server is a child process the call starts and stops.
//! - `http` (Streamable HTTP): the server is at the `endpoint`; `headers` go on every request.
//!
//! The task's output is the JSON-RPC `result`. Any failure (the server could not be started or reached, the server
//! answered with an error, the call ran past its `timeout`) is a `runtime` error.
use crate::context::Context;
use crate::executor::{Error, Executor, Result};
use async_trait::async_trait;
use rmcp::ServiceExt as _;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, GetPromptRequestParams,
    Implementation, ProtocolVersion, ReadResourceRequestParams,
};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// The protocol revision asked for when the task names none.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// The methods a task may call (the specification's list).
const METHODS: &[&str] = &[
    "tools/list",
    "tools/call",
    "prompts/list",
    "prompts/get",
    "resources/list",
    "resources/read",
    "resources/templates/list",
];

/// Runs `call: mcp` tasks.
#[derive(Debug, Default)]
pub struct McpExecutor;

impl McpExecutor {
    /// An executor for `call: mcp`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Executor for McpExecutor {
    async fn exec(
        &self,
        task_name: &str,
        params: &Value,
        _ctx: &Context,
        _streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> Result<Value> {
        let request = Request::parse(params).map_err(|detail| runtime_error(task_name, &detail))?;
        let outcome = match request.timeout {
            // Dropping the call on a timeout drops the connection, which stops a child process server.
            Some(limit) => tokio::time::timeout(limit, request.run())
                .await
                .unwrap_or_else(|_| Err(format!("the MCP call timed out after {limit:?}"))),
            None => request.run().await,
        };
        outcome.map_err(|detail| runtime_error(task_name, &detail))
    }

    /// An MCP server is a side effect (`tools/call` does things), so its answers are not cached.
    fn cacheable(&self) -> bool {
        false
    }
}

/// The `runtime` error the specification asks for, as the problem document the rest of the engine's executors raise.
fn runtime_error(task_name: &str, detail: &str) -> Error {
    let problem = json!({
        "type": crate::api::error_types::RUNTIME,
        "status": 500,
        "title": "Runtime Error",
        "detail": detail,
        "instance": format!("/do/0/{task_name}"),
    });
    Error::Execution {
        message: problem.to_string(),
    }
}

/// A task's arguments, checked.
struct Request {
    protocol_version: String,
    method: String,
    parameters: Value,
    timeout: Option<Duration>,
    transport: TransportConfig,
    client_name: String,
    client_version: String,
}

enum TransportConfig {
    Stdio {
        command: String,
        arguments: Vec<String>,
        environment: HashMap<String, String>,
    },
    Http {
        endpoint: String,
        headers: HashMap<String, String>,
        bearer: Option<String>,
    },
}

type Client = RunningService<RoleClient, ClientConfig>;

impl Request {
    fn parse(params: &Value) -> std::result::Result<Self, String> {
        let method = params
            .get("method")
            .and_then(Value::as_str)
            .ok_or("no `method` in the MCP call")?
            .to_string();
        if !METHODS.contains(&method.as_str()) {
            return Err(format!(
                "unsupported MCP method `{method}`; use one of {}",
                METHODS.join(", ")
            ));
        }
        let parameters = match params.get("parameters") {
            None | Some(Value::Null) => json!({}),
            // `parameters` may also be a string: JSON text, which a runtime expression often produces.
            Some(Value::String(text)) => serde_json::from_str(text)
                .map_err(|e| format!("`parameters` is a string that is not JSON: {e}"))?,
            Some(other) => other.clone(),
        };
        let protocol_version = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION)
            .to_string();
        let timeout = params
            .get("timeout")
            .filter(|v| !v.is_null())
            .map(parse_timeout)
            .transpose()?;
        let client = params.get("client");
        let client_field = |field: &str, default: &str| {
            client
                .and_then(|c| c.get(field))
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_string()
        };
        Ok(Self {
            protocol_version,
            method,
            parameters,
            timeout,
            transport: parse_transport(params.get("transport"))?,
            client_name: client_field("name", "jackdaw"),
            client_version: client_field("version", env!("CARGO_PKG_VERSION")),
        })
    }

    /// Connects (the SDK sends `initialize` and `initialized`), sends the request, and returns the result.
    async fn run(&self) -> std::result::Result<Value, String> {
        let version: ProtocolVersion = serde_json::from_value(json!(self.protocol_version))
            .map_err(|e| format!("invalid `protocolVersion`: {e}"))?;
        let info = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new(&self.client_name, &self.client_version),
        )
        .with_protocol_version(version);
        let client = match &self.transport {
            TransportConfig::Stdio {
                command,
                arguments,
                environment,
            } => {
                let mut process = Command::new(command);
                process.args(arguments).envs(environment);
                // The server's logs go to stderr; they are not part of the protocol.
                let (transport, _) = TokioChildProcess::builder(process)
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("could not start the MCP server `{command}`: {e}"))?;
                info.serve(transport).await
            }
            TransportConfig::Http {
                endpoint,
                headers,
                bearer,
            } => {
                let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint.as_str());
                let mut custom = HashMap::new();
                for (name, value) in headers {
                    custom.insert(
                        http::HeaderName::from_bytes(name.as_bytes())
                            .map_err(|e| format!("invalid header name `{name}`: {e}"))?,
                        http::HeaderValue::from_str(value)
                            .map_err(|e| format!("invalid value for the header `{name}`: {e}"))?,
                    );
                }
                config = config.custom_headers(custom);
                if let Some(token) = bearer {
                    config = config.auth_header(token.as_str());
                }
                info.serve(StreamableHttpClientTransport::from_config(config))
                    .await
            }
        }
        .map_err(|e| format!("MCP initialize failed: {e}"))?;
        let outcome = self.send(&client).await;
        // Closes the transport; a child process server is stopped.
        let _ = client.cancel().await;
        outcome
    }

    async fn send(&self, client: &Client) -> std::result::Result<Value, String> {
        let method = self.method.as_str();
        let outcome = match method {
            "tools/list" => reply(client.list_tools(Some(self.typed()?)).await),
            "tools/call" => reply(client.call_tool(self.typed::<CallToolRequestParams>()?).await),
            "prompts/list" => reply(client.list_prompts(Some(self.typed()?)).await),
            "prompts/get" => reply(client.get_prompt(self.typed::<GetPromptRequestParams>()?).await),
            "resources/list" => reply(client.list_resources(Some(self.typed()?)).await),
            "resources/read" => reply(
                client
                    .read_resource(self.typed::<ReadResourceRequestParams>()?)
                    .await,
            ),
            "resources/templates/list" => {
                reply(client.list_resource_templates(Some(self.typed()?)).await)
            }
            other => Err(format!("unsupported MCP method `{other}`")),
        };
        outcome.map_err(|e| format!("MCP `{method}` failed: {e}"))
    }

    fn typed<T: DeserializeOwned>(&self) -> std::result::Result<T, String> {
        serde_json::from_value(self.parameters.clone())
            .map_err(|e| format!("invalid `parameters` for `{}`: {e}", self.method))
    }
}

fn reply<T: Serialize>(
    result: std::result::Result<T, ServiceError>,
) -> std::result::Result<Value, String> {
    let result = result.map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|e| format!("the result is not JSON: {e}"))
}

fn parse_timeout(value: &Value) -> std::result::Result<Duration, String> {
    match value {
        Value::String(iso) => crate::durableengine::timeout::parse_iso8601_duration(iso)
            .map_err(|e| format!("invalid `timeout`: {e}")),
        Value::Object(fields) => {
            let unit = |name: &str, millis: u64| {
                fields.get(name).and_then(Value::as_u64).unwrap_or(0) * millis
            };
            Ok(Duration::from_millis(
                unit("days", 86_400_000)
                    + unit("hours", 3_600_000)
                    + unit("minutes", 60_000)
                    + unit("seconds", 1000)
                    + unit("milliseconds", 1),
            ))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
            Err("`timeout` is a duration object or an ISO 8601 string".to_string())
        }
    }
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn string_map(
    value: Option<&Value>,
    what: &str,
) -> std::result::Result<HashMap<String, String>, String> {
    match value {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                scalar(v)
                    .map(|s| (k.clone(), s))
                    .ok_or_else(|| format!("`{what}.{k}` is not a string"))
            })
            .collect(),
        Some(_) => Err(format!("`{what}` is a map of strings")),
    }
}

fn parse_transport(transport: Option<&Value>) -> std::result::Result<TransportConfig, String> {
    let transport = transport.ok_or("no `transport` in the MCP call")?;
    if let Some(stdio) = transport.get("stdio") {
        let command = stdio
            .get("command")
            .and_then(Value::as_str)
            .ok_or("no `command` in the stdio transport")?
            .to_string();
        let arguments = match stdio.get("arguments") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    scalar(item).ok_or_else(|| "`arguments` of the stdio transport are strings".to_string())
                })
                .collect::<std::result::Result<_, _>>()?,
            Some(_) => return Err("`arguments` of the stdio transport is a list".to_string()),
        };
        return Ok(TransportConfig::Stdio {
            command,
            arguments,
            environment: string_map(stdio.get("environment"), "environment")?,
        });
    }
    if let Some(http) = transport.get("http") {
        let endpoint = http
            .get("endpoint")
            .ok_or("no `endpoint` in the http transport")?;
        let (endpoint, mut bearer) = match endpoint {
            Value::String(uri) => (uri.clone(), None),
            Value::Object(fields) => {
                let uri = fields
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or("no `uri` in the endpoint")?
                    .to_string();
                (uri, bearer_token(fields.get("authentication"))?)
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
                return Err("`endpoint` is a URI or an object with a `uri`".to_string());
            }
        };
        let mut headers = string_map(http.get("headers"), "headers")?;
        // The SDK keeps `Authorization` apart from the custom headers.
        let authorization = headers
            .keys()
            .find(|name| name.eq_ignore_ascii_case("authorization"))
            .cloned();
        if let Some(name) = authorization {
            let value = headers.remove(&name).unwrap_or_default();
            bearer = Some(
                value
                    .strip_prefix("Bearer ")
                    .ok_or("an `Authorization` header must be `Bearer <token>`")?
                    .to_string(),
            );
        }
        return Ok(TransportConfig::Http {
            endpoint,
            headers,
            bearer,
        });
    }
    Err("the `transport` is `stdio` or `http`".to_string())
}

/// The token of an endpoint's inline `bearer` authentication.
fn bearer_token(auth: Option<&Value>) -> std::result::Result<Option<String>, String> {
    let Some(auth) = auth.filter(|a| !a.is_null()) else {
        return Ok(None);
    };
    let token = auth
        .get("bearer")
        .and_then(|bearer| bearer.get("token"))
        .and_then(Value::as_str)
        .ok_or("the endpoint's authentication of an MCP call is `bearer` with a `token`")?;
    Ok(Some(token.to_string()))
}
