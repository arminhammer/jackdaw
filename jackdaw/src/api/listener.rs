//! Listeners: the engine serves HTTP routes from an OpenAPI document, or a gRPC service from a `.proto` file, and each
//! request starts the workflow bound to its operation or method.
//!
//! ```no_run
//! use jackdaw::api::Listener;
//! use jackdaw::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct Pair {
//!     a: i64,
//!     b: i64,
//! }
//!
//! #[derive(Serialize, Deserialize)]
//! struct Sum {
//!     sum: i64,
//! }
//!
//! #[workflow(namespace = "calc", name = "add", version = "1.0.0")]
//! struct Add;
//!
//! #[workflow_methods]
//! impl Add {
//!     #[run]
//!     async fn run(_ctx: &mut WorkflowContext<Self>, pair: Pair) -> WorkflowResult<Sum> {
//!         Ok(Sum { sum: pair.a + pair.b })
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let spec = std::fs::read_to_string("openapi.yaml").expect("the spec");
//! let engine = Engine::builder()
//!     .workflow::<Add>()
//!     .listener(Listener::openapi("api", &spec, "127.0.0.1:8080")?.bind("addNumbers", Add::run))
//!     .build()?;
//! println!("serving on {:?}", engine.listener_addr("api"));
//! # Ok(())
//! # }
//! ```
//!
//! A request is first checked against its operation in the document: required parameters, the schemas of its path,
//! query and header parameters, and its JSON request body's schema (`$ref`s into `components` resolve; an `openapi:
//! 3.0` document's schemas are read as JSON Schema draft 4, a 3.1 one's as 2020-12). One that does not match is a
//! `400` whose body lists the problems, and no workflow starts.
//!
//! A request's input is its JSON body; path and query parameters are added to it (a body-less request is just its
//! parameters). The workflow's output is the response body. Input the workflow cannot read is a `400`, a workflow
//! that fails a `500`, and an engine that is shutting down a `503`.
//!
//! Listeners serve plain HTTP and gRPC (h2c). For TLS, put a reverse proxy or a sidecar in front of them. [`Engine::shutdown`] stops the listeners first and
//! waits for the requests they are serving; dropping the engine closes them at once.
use super::engine::Engine;
use super::error::Error;
use super::task::BoxFuture;
use super::workflow::Runnable;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

/// Starts the bound workflow with a request's input and returns its output.
type Binding = Arc<dyn Fn(Engine, Value) -> BoxFuture<'static, Result<Value, Error>> + Send + Sync>;

/// HTTP routes or a gRPC service the engine serves. Register it with
/// [`EngineBuilder::listener`](super::EngineBuilder::listener).
pub struct Listener {
    pub(crate) name: String,
    addr: String,
    kind: Kind,
    bindings: BTreeMap<String, Binding>,
}

enum Kind {
    #[cfg(feature = "http-listener")]
    OpenApi(Vec<Route>),
    #[cfg(feature = "grpc-listener")]
    Grpc {
        service: prost_reflect::ServiceDescriptor,
        files: prost_types::FileDescriptorSet,
    },
}

/// One operation of an OpenAPI document.
#[cfg(feature = "http-listener")]
struct Route {
    path: String,
    method: String,
    operation: String,
    rules: Arc<RequestRules>,
}

/// What a request to one operation must look like: its parameters and its JSON body, from the OpenAPI document.
#[cfg(feature = "http-listener")]
struct RequestRules {
    parameters: Vec<ParameterRule>,
    body_required: bool,
    body: Option<jsonschema::Validator>,
}

#[cfg(feature = "http-listener")]
struct ParameterRule {
    name: String,
    /// `path`, `query` or `header` (cookie parameters are not checked).
    location: String,
    required: bool,
    schema: Option<jsonschema::Validator>,
}

#[cfg(feature = "http-listener")]
impl RequestRules {
    /// Reads the rules of `operation` (under `path_item`) from `document`.
    fn new(
        document: &Value,
        path_item: &Value,
        operation: &Value,
        what: &str,
    ) -> Result<Self, Error> {
        let draft = if document
            .get("openapi")
            .and_then(Value::as_str)
            .is_some_and(|version| version.starts_with("3.0") || version.starts_with('2'))
        {
            jsonschema::Draft::Draft4
        } else {
            jsonschema::Draft::Draft202012
        };
        // `$ref`s in a schema point into the document's `components`, so each schema is compiled with them beside it.
        let compile = |schema: &Value, of: &str| -> Result<jsonschema::Validator, Error> {
            let mut root = resolve(document, schema).clone();
            if let (Value::Object(fields), Some(components)) =
                (&mut root, document.get("components"))
            {
                fields
                    .entry("components")
                    .or_insert_with(|| components.clone());
            }
            jsonschema::options()
                .with_draft(draft)
                .build(&root)
                .map_err(|e| {
                    Error::invalid(format!("{what}: the schema of {of} is not valid: {e}"))
                })
        };

        // The path item's parameters, then the operation's, which override one with the same name and location.
        let mut parameters: Vec<ParameterRule> = Vec::new();
        for parameter in [path_item, operation]
            .into_iter()
            .filter_map(|item| item.get("parameters").and_then(Value::as_array))
            .flatten()
        {
            let parameter = resolve(document, parameter);
            let (Some(name), Some(location)) = (
                parameter.get("name").and_then(Value::as_str),
                parameter.get("in").and_then(Value::as_str),
            ) else {
                continue;
            };
            let rule = ParameterRule {
                name: name.to_string(),
                location: location.to_string(),
                required: location == "path"
                    || parameter.get("required").and_then(Value::as_bool) == Some(true),
                schema: parameter
                    .get("schema")
                    .map(|schema| compile(schema, &format!("{location} parameter `{name}`")))
                    .transpose()?,
            };
            parameters
                .retain(|other| !(other.name == rule.name && other.location == rule.location));
            parameters.push(rule);
        }

        let body = operation
            .get("requestBody")
            .map(|body| resolve(document, body));
        let body_required = body
            .and_then(|body| body.get("required"))
            .and_then(Value::as_bool)
            == Some(true);
        let body_schema = body
            .and_then(|body| body.pointer("/content/application~1json/schema"))
            .map(|schema| compile(schema, "the request body"))
            .transpose()?;
        Ok(Self {
            parameters,
            body_required,
            body: body_schema,
        })
    }

    /// The ways a request breaks the rules; empty when it follows them.
    fn check(
        &self,
        path: &axum::extract::RawPathParams,
        query: &BTreeMap<String, String>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Vec<String> {
        let mut problems = Vec::new();
        for rule in &self.parameters {
            let value = match rule.location.as_str() {
                "path" => path
                    .iter()
                    .find(|(name, _)| *name == rule.name)
                    .map(|(_, value)| value.to_string()),
                "query" => query.get(&rule.name).cloned(),
                "header" => headers
                    .get(&rule.name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
                _ => continue,
            };
            match (value, &rule.schema) {
                (None, _) if rule.required => problems.push(format!(
                    "the {} parameter `{}` is required",
                    rule.location, rule.name
                )),
                (Some(text), Some(schema)) => {
                    // Parameters arrive as text: `42` is valid for a string schema and for an integer one.
                    let spelled = serde_json::from_str::<Value>(&text).ok();
                    let as_text = Value::String(text);
                    if !schema.is_valid(&as_text)
                        && !spelled.as_ref().is_some_and(|value| schema.is_valid(value))
                    {
                        let shown = spelled.as_ref().unwrap_or(&as_text);
                        let reasons: Vec<String> = schema
                            .iter_errors(shown)
                            .map(|error| error.to_string())
                            .collect();
                        problems.push(format!(
                            "the {} parameter `{}`: {}",
                            rule.location,
                            rule.name,
                            reasons.join("; ")
                        ));
                    }
                }
                (None, _) | (Some(_), None) => {}
            }
        }
        if body.is_empty() {
            if self.body_required {
                problems.push("a request body is required".to_string());
            }
        } else if let Some(schema) = &self.body
            && let Ok(value) = serde_json::from_slice::<Value>(body)
        {
            // A body that is not JSON is reported when the input is read.
            for error in schema.iter_errors(&value) {
                let at = error.instance_path().to_string();
                problems.push(if at.is_empty() {
                    format!("the request body: {error}")
                } else {
                    format!("the request body at `{at}`: {error}")
                });
            }
        }
        problems
    }
}

/// `value`, or what its local `$ref` (`#/components/...`) points to in `document`.
#[cfg(feature = "http-listener")]
fn resolve<'a>(document: &'a Value, value: &'a Value) -> &'a Value {
    let mut value = value;
    // A chain of references, but not a cycle.
    for _ in 0..8 {
        match value
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| reference.strip_prefix('#'))
            .and_then(|pointer| document.pointer(pointer))
        {
            Some(target) => value = target,
            None => break,
        }
    }
    value
}

impl fmt::Debug for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Listener")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("bound", &self.bindings.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// Serves the operations of an OpenAPI document (YAML or JSON) on `addr` (`host:port`; port `0` lets the OS pick,
    /// see [`Engine::listener_addr`]). Bind its operations, by `operationId`, with [`bind`](Self::bind).
    ///
    /// # Errors
    /// `Error::Invalid` if the document does not parse, has no operations with an `operationId`, or has a parameter or
    /// request body schema that is not valid JSON Schema.
    #[cfg(feature = "http-listener")]
    #[cfg_attr(docsrs, doc(cfg(feature = "http-listener")))]
    pub fn openapi(
        name: impl Into<String>,
        spec: &str,
        addr: impl Into<String>,
    ) -> Result<Self, Error> {
        const METHODS: [&str; 8] = [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ];
        let name = name.into();
        let document: Value = serde_saphyr::from_str(spec).map_err(|e| {
            Error::invalid(format!(
                "listener `{name}`: the OpenAPI document does not parse: {e}"
            ))
        })?;
        let mut routes = Vec::new();
        for (path, item) in document
            .get("paths")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            for method in METHODS {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                if let Some(id) = operation.get("operationId").and_then(Value::as_str) {
                    let rules = RequestRules::new(
                        &document,
                        item,
                        operation,
                        &format!("listener `{name}`: operation `{id}`"),
                    )?;
                    routes.push(Route {
                        path: path.clone(),
                        method: method.to_string(),
                        operation: id.to_string(),
                        rules: Arc::new(rules),
                    });
                }
            }
        }
        if routes.is_empty() {
            return Err(Error::invalid(format!(
                "listener `{name}`: the OpenAPI document has no operations with an `operationId`"
            )));
        }
        Ok(Self {
            name,
            addr: addr.into(),
            kind: Kind::OpenApi(routes),
            bindings: BTreeMap::new(),
        })
    }

    /// Serves `service` (its full name, `package.Service`, or just `Service`) from a `.proto` file on `addr`. Bind its
    /// methods, by name, with [`bind`](Self::bind). Messages convert to and from the workflows' JSON with the
    /// protobuf JSON mapping (field names in `lowerCamelCase`, 64-bit integers as strings).
    ///
    /// # Errors
    /// `Error::Invalid` if the file does not compile or has no such service.
    #[cfg(feature = "grpc-listener")]
    #[cfg_attr(docsrs, doc(cfg(feature = "grpc-listener")))]
    pub fn grpc(
        name: impl Into<String>,
        proto: impl AsRef<std::path::Path>,
        service: &str,
        addr: impl Into<String>,
    ) -> Result<Self, Error> {
        use prost::Message as _;
        let name = name.into();
        let proto = proto.as_ref();
        let (Some(dir), Some(file)) = (proto.parent(), proto.file_name()) else {
            return Err(Error::invalid(format!(
                "listener `{name}`: `{}` is not a file",
                proto.display()
            )));
        };
        let dir = if dir.as_os_str().is_empty() {
            std::path::Path::new(".")
        } else {
            dir
        };
        let files = protox::compile([file], [dir]).map_err(|e| {
            Error::invalid(format!(
                "listener `{name}`: {} does not compile: {e}",
                proto.display()
            ))
        })?;
        let pool = prost_reflect::DescriptorPool::decode(files.encode_to_vec().as_slice())
            .map_err(|e| Error::invalid(format!("listener `{name}`: {e}")))?;
        let found = pool.get_service_by_name(service).or_else(|| {
            pool.services()
                .find(|candidate| candidate.name() == service)
        });
        let Some(service) = found else {
            return Err(Error::invalid(format!(
                "listener `{name}`: {} has no service `{service}`",
                proto.display()
            )));
        };
        Ok(Self {
            name,
            addr: addr.into(),
            kind: Kind::Grpc { service, files },
            bindings: BTreeMap::new(),
        })
    }

    /// Starts `workflow` for each request to `operation` (an OpenAPI `operationId`, or a gRPC method name) and
    /// answers with its output.
    #[must_use]
    pub fn bind<R>(mut self, operation: impl Into<String>, workflow: R) -> Self
    where
        R: Runnable + Clone + Send + Sync + 'static,
        R::In: DeserializeOwned,
        R::Out: Serialize,
    {
        let binding: Binding = Arc::new(move |engine: Engine, input: Value| {
            let workflow = workflow.clone();
            Box::pin(async move {
                let input: R::In = serde_json::from_value(input).map_err(|e| {
                    Error::invalid(format!("the request is not the workflow's input: {e}"))
                })?;
                let output = engine.execute(workflow, input).await?.await?;
                serde_json::to_value(&output).map_err(|e| {
                    Error::Engine(format!(
                        "the workflow's output could not be serialized: {e}"
                    ))
                })
            })
        });
        self.bindings.insert(operation.into(), binding);
        self
    }

    /// The operations or methods of the document or service.
    fn operations(&self) -> Vec<String> {
        match &self.kind {
            #[cfg(feature = "http-listener")]
            Kind::OpenApi(routes) => routes.iter().map(|route| route.operation.clone()).collect(),
            #[cfg(feature = "grpc-listener")]
            Kind::Grpc { service, .. } => service
                .methods()
                .map(|method| method.name().to_string())
                .collect(),
        }
    }

    /// Binds the socket and starts serving. Called by `EngineBuilder::build`, inside a tokio runtime.
    pub(crate) fn start(
        self,
        engine: std::sync::Weak<super::runtime::Shared>,
    ) -> Result<Serving, Error> {
        let known = self.operations();
        if let Some(unknown) = self.bindings.keys().find(|bound| !known.contains(bound)) {
            return Err(Error::invalid(format!(
                "listener `{}` binds `{unknown}`, which it does not have (it has: {})",
                self.name,
                known.join(", ")
            )));
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(Error::invalid(format!(
                "listener `{}`: listeners are served on a tokio runtime; build the engine inside one",
                self.name
            )));
        }
        let socket = std::net::TcpListener::bind(&self.addr)
            .and_then(|socket| socket.set_nonblocking(true).map(|()| socket))
            .map_err(|e| {
                Error::invalid(format!(
                    "listener `{}` cannot listen on {}: {e}",
                    self.name, self.addr
                ))
            })?;
        let addr = socket
            .local_addr()
            .map_err(|e| Error::Engine(format!("listener `{}`: {e}", self.name)))?;
        let socket = tokio::net::TcpListener::from_std(socket)
            .map_err(|e| Error::Engine(format!("listener `{}`: {e}", self.name)))?;
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let bindings = self.bindings;
        let task = match self.kind {
            #[cfg(feature = "http-listener")]
            Kind::OpenApi(routes) => {
                super::observe::spawn(serve_http(socket, routes, bindings, engine, stopped))
            }
            #[cfg(feature = "grpc-listener")]
            Kind::Grpc { service, files } => super::observe::spawn(serve_grpc(
                socket, service, files, bindings, engine, stopped,
            )),
        };
        Ok(Serving {
            name: self.name,
            addr,
            stop: Mutex::new(Some(stop)),
            task: Mutex::new(Some(task)),
        })
    }
}

/// A listener the engine is serving.
pub(crate) struct Serving {
    pub name: String,
    pub addr: SocketAddr,
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Serving {
    /// Stops accepting and waits (up to `grace`) for the requests in flight.
    pub(crate) async fn stop(&self, grace: std::time::Duration) {
        if let Some(stop) = lock(&self.stop).take() {
            let _ = stop.send(());
        }
        let task = lock(&self.task).take();
        if let Some(mut task) = task
            && tokio::time::timeout(grace, &mut task).await.is_err()
        {
            task.abort();
        }
    }

    /// Closes the socket at once (the engine was dropped).
    pub(crate) fn abort(&self) {
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs a request's workflow, if the engine is still there.
async fn dispatch(
    binding: &Binding,
    engine: &std::sync::Weak<super::runtime::Shared>,
    input: Value,
) -> Result<Value, Error> {
    let Some(shared) = engine.upgrade() else {
        return Err(Error::Engine("the engine is shutting down".to_string()));
    };
    binding(Engine::from_shared(shared), input).await
}

#[cfg(feature = "http-listener")]
async fn serve_http(
    socket: tokio::net::TcpListener,
    routes: Vec<Route>,
    bindings: BTreeMap<String, Binding>,
    engine: std::sync::Weak<super::runtime::Shared>,
    stopped: tokio::sync::oneshot::Receiver<()>,
) {
    use axum::extract::{Query, RawPathParams};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{MethodFilter, on};

    let mut router = axum::Router::new();
    for route in routes {
        let Some(binding) = bindings.get(&route.operation).cloned() else {
            continue;
        };
        let filter = match route.method.as_str() {
            "get" => MethodFilter::GET,
            "put" => MethodFilter::PUT,
            "post" => MethodFilter::POST,
            "delete" => MethodFilter::DELETE,
            "options" => MethodFilter::OPTIONS,
            "head" => MethodFilter::HEAD,
            "patch" => MethodFilter::PATCH,
            _ => MethodFilter::TRACE,
        };
        let engine = engine.clone();
        let rules = Arc::clone(&route.rules);
        let handler = move |params: RawPathParams,
                            Query(query): Query<BTreeMap<String, String>>,
                            headers: axum::http::HeaderMap,
                            body: axum::body::Bytes| {
            let (binding, engine, rules) =
                (Arc::clone(&binding), engine.clone(), Arc::clone(&rules));
            async move {
                let problems = rules.check(&params, &query, &headers, &body);
                if !problems.is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        axum::Json(serde_json::json!({
                            "error": "the request does not match the OpenAPI document",
                            "problems": problems,
                        })),
                    )
                        .into_response();
                }
                let input = match crate::listeners::request_input(&params, query, &body) {
                    Ok(input) => input,
                    Err(message) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            axum::Json(serde_json::json!({ "error": message })),
                        )
                            .into_response();
                    }
                };
                match dispatch(&binding, &engine, input).await {
                    Ok(output) => (StatusCode::OK, axum::Json(output)).into_response(),
                    Err(error) => {
                        let status = if matches!(error, Error::Invalid(_)) {
                            StatusCode::BAD_REQUEST
                        } else if matches!(&error, Error::Engine(message) if message.contains("shutting down"))
                        {
                            StatusCode::SERVICE_UNAVAILABLE
                        } else {
                            StatusCode::INTERNAL_SERVER_ERROR
                        };
                        (
                            status,
                            axum::Json(serde_json::json!({ "error": error.to_string() })),
                        )
                            .into_response()
                    }
                }
            }
        };
        router = router.route(&route.path, on(filter, handler));
    }
    let _ = axum::serve(socket, router)
        .with_graceful_shutdown(async move {
            let _ = stopped.await;
        })
        .await;
}

#[cfg(feature = "grpc-listener")]
async fn serve_grpc(
    socket: tokio::net::TcpListener,
    service: prost_reflect::ServiceDescriptor,
    files: prost_types::FileDescriptorSet,
    bindings: BTreeMap<String, Binding>,
    engine: std::sync::Weak<super::runtime::Shared>,
    stopped: tokio::sync::oneshot::Receiver<()>,
) {
    use crate::listeners::Listener as _;
    use crate::listeners::grpc::{GrpcListener, MethodHandler};

    let mut handlers: std::collections::HashMap<String, MethodHandler> =
        std::collections::HashMap::new();
    for method in service.methods() {
        let Some(binding) = bindings.get(method.name()).cloned() else {
            continue;
        };
        let engine = engine.clone();
        let output = method.output();
        let handler: MethodHandler = Arc::new(move |request: prost_reflect::DynamicMessage| {
            let (binding, engine, output) = (Arc::clone(&binding), engine.clone(), output.clone());
            Box::pin(async move {
                let input = serde_json::to_value(&request).map_err(|e| {
                    crate::listeners::Error::Execution {
                        message: format!("the request could not be read: {e}"),
                    }
                })?;
                let result = dispatch(&binding, &engine, input).await.map_err(|e| {
                    crate::listeners::Error::Execution {
                        message: e.to_string(),
                    }
                })?;
                prost_reflect::DynamicMessage::deserialize(output, result).map_err(|e| {
                    crate::listeners::Error::Execution {
                        message: format!(
                            "the workflow's output does not fit the method's output message: {e}"
                        ),
                    }
                })
            })
        });
        handlers.insert(method.name().to_string(), handler);
    }
    let addr = socket
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_default();
    let server = GrpcListener::from_descriptors(addr, service, files, handlers);
    if server.serve_on(socket).await.is_ok() {
        let _ = stopped.await;
        let _ = server.stop().await;
    }
}
