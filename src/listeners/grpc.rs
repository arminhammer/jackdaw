use super::{Listener, Result};
use async_trait::async_trait;
use bytes::{Buf, Bytes};
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::service::Service as HyperService;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, ServiceDescriptor};
use prost_types::FileDescriptorSet;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::sync::RwLock;
use tonic::{Status, server::NamedService};
use tonic_reflection::server::Builder as ReflectionBuilder;
use tower::Service as TowerService;

// Type alias for boxed body (tonic 0.14+ made BoxBody private)
type BoxBody = UnsyncBoxBody<Bytes, Status>;

/// gRPC listener for handling proto-based service requests
pub struct GrpcListener {
    /// Bind address (e.g., "localhost:50051")
    bind_addr: String,

    /// Service descriptor
    service_descriptor: ServiceDescriptor,

    /// File descriptor set for reflection support
    file_descriptor_set: FileDescriptorSet,

    /// Method handlers: ``method_name`` -> handler function
    /// For multi-method servers, this contains all handlers
    /// Using ``RwLock`` to allow adding methods dynamically
    method_handlers: Arc<
        RwLock<
            std::collections::HashMap<
                String,
                Arc<dyn Fn(DynamicMessage) -> Result<DynamicMessage> + Send + Sync>,
            >,
        >,
    >,

    /// Server handle for shutdown
    shutdown_tx: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

#[allow(dead_code)]
impl GrpcListener {
    /// Add a method handler to an existing listener
    /// This allows adding new methods to an already-running server
    ///
    /// # Errors
    /// Returns an error if the provided `method_name` does not exist in the service descriptor.
    pub async fn add_method(
        &self,
        method_name: String,
        handler: Arc<dyn Fn(DynamicMessage) -> Result<DynamicMessage> + Send + Sync>,
    ) -> Result<()> {
        // Validate that the method exists in the service
        if !self
            .service_descriptor
            .methods()
            .any(|m| m.name() == method_name)
        {
            return Err(super::Error::Listener {
                message: format!(
                    "Method {} not found in service {}",
                    method_name,
                    self.service_descriptor.full_name()
                ),
            });
        }

        // Add to handlers map
        let mut handlers = self.method_handlers.write().await;
        handlers.insert(method_name.clone(), handler);

        tracing::info!(
            "Added method {} to gRPC listener on {}",
            method_name,
            self.bind_addr
        );
        Ok(())
    }

    /// Create a new gRPC listener with multiple method handlers
    /// This allows a single server to handle multiple methods on the same port
    ///
    /// # Errors
    /// Returns an error if:
    /// - The provided proto file cannot be compiled or encoded.
    /// - The compiled descriptors cannot be decoded into a `DescriptorPool`.
    /// - The requested `service_name` is not found in the proto descriptors.
    /// - Any of the provided `method_handlers` refer to a method name not present in the service.
    pub fn new_multi_method(
        bind_addr: String,
        proto_path: &str,
        service_name: &str,
        method_handlers: std::collections::HashMap<
            String,
            Arc<dyn Fn(DynamicMessage) -> Result<DynamicMessage> + Send + Sync>,
        >,
    ) -> Result<Self> {
        // Compile proto file and build descriptor pool
        let file_descriptor_set = protox::compile([proto_path], ["."])?;
        let mut buf = Vec::new();
        file_descriptor_set.encode(&mut buf)?;
        let descriptor_pool = DescriptorPool::decode(buf.as_slice())?;

        // Get service descriptor
        let service_descriptor = descriptor_pool
            .get_service_by_name(service_name)
            .ok_or_else(|| super::Error::Listener {
                message: format!("Service {service_name} not found in proto"),
            })?;

        // Validate that all methods exist in the service
        for method_name in method_handlers.keys() {
            if !service_descriptor
                .methods()
                .any(|m| m.name() == method_name)
            {
                return Err(super::Error::Listener {
                    message: format!("Method {method_name} not found in service {service_name}"),
                });
            }
        }

        Ok(Self {
            bind_addr,
            service_descriptor,
            file_descriptor_set,
            method_handlers: Arc::new(RwLock::new(method_handlers)),
            shutdown_tx: Arc::new(RwLock::new(None)),
        })
    }
}

#[async_trait]
impl Listener for GrpcListener {
    async fn start(&self) -> Result<()> {
        let method_names: Vec<String> = {
            let handlers = self.method_handlers.read().await;
            handlers.keys().cloned().collect()
        };
        tracing::info!(
            "Starting gRPC listener on {} for {}/{:?}",
            self.bind_addr,
            self.service_descriptor.full_name(),
            method_names
        );

        // Clone what we need for the server task
        let bind_addr = self.bind_addr.clone();
        let method_handlers = self.method_handlers.clone();
        let service_descriptor = self.service_descriptor.clone();
        let file_descriptor_set = self.file_descriptor_set.clone();

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Store shutdown sender
        {
            let mut tx_lock = self.shutdown_tx.write().await;
            *tx_lock = Some(shutdown_tx);
        }

        // Spawn gRPC server in background
        tokio::spawn(async move {
            println!("  Spawning gRPC server task for {bind_addr}");

            // Create a multi-method dynamic gRPC service handler
            let service = MultiMethodGrpcService {
                method_handlers,
                service_descriptor,
            };

            // Parse bind address
            let addr: std::net::SocketAddr = match bind_addr.parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("  Failed to parse bind address {bind_addr}: {e}");
                    return;
                }
            };

            println!("  gRPC server about to listen on {addr}");
            tracing::info!("gRPC server listening on {}", addr);

            let service_wrapper = service.into_service();

            // Build reflection service from file descriptor set
            let reflection_service = match ReflectionBuilder::configure()
                .register_encoded_file_descriptor_set(
                    file_descriptor_set.encode_to_vec().as_slice(),
                )
                .build_v1()
            {
                Ok(service) => service,
                Err(e) => {
                    eprintln!("  Failed to build reflection service: {e}");
                    tracing::error!("Failed to build reflection service: {e}");
                    return;
                }
            };

            println!("  Starting tonic server on {addr} with reflection support");

            // Wrap reflection service to convert its body type to BoxBody
            let reflection_adapted = ReflectionAdapter {
                inner: reflection_service,
            };

            // Combine reflection service and our custom dynamic router into one service
            // This service will route based on the request path
            let combined_service = CombinedService {
                reflection: reflection_adapted,
                dynamic_router: service_wrapper,
            };

            // Use hyper_util's TokioIo and serve connection directly
            use hyper_util::rt::TokioIo;
            use hyper_util::server::conn::auto;

            let tcp_listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => listener,
                Err(e) => {
                    eprintln!("  Failed to bind to {addr}: {e}");
                    tracing::error!("Failed to bind to {addr}: {e}");
                    return;
                }
            };

            println!("  gRPC server listening on {addr}");

            let result = async move {
                loop {
                    let (tcp_stream, _remote_addr) = match tcp_listener.accept().await {
                        Ok(conn) => conn,
                        Err(e) => {
                            eprintln!("  Failed to accept connection: {e}");
                            continue;
                        }
                    };

                    let io = TokioIo::new(tcp_stream);

                    // Use our HyperAdapter to convert body types from Incoming to BoxBody
                    let svc = HyperAdapter {
                        inner: combined_service.clone(),
                    };

                    tokio::task::spawn(async move {
                        if let Err(err) = auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                            .serve_connection(io, svc)
                            .await
                        {
                            eprintln!("  Error serving connection: {:?}", err);
                        }
                    });
                }
            };

            tokio::select! {
                _ = result => {},
                _ = shutdown_rx => {
                    println!("  gRPC server on {addr} received shutdown signal");
                }
            }
            println!("  gRPC server on {addr} exited cleanly");
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        tracing::info!("Stopping gRPC listener on {}", self.bind_addr);

        let mut shutdown = self.shutdown_tx.write().await;
        if let Some(tx) = shutdown.take() {
            let _ = tx.send(());
        }

        Ok(())
    }

    fn get_endpoint(&self) -> String {
        // Note: This is synchronous but we need to read from RwLock
        // In practice, this should only be called after initialization
        // We'll use blocking read which is OK for this use case
        let methods: Vec<String> = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let handlers = self.method_handlers.read().await;
                handlers.keys().cloned().collect()
            })
        });
        format!(
            "grpc://{}/{}/[{}]",
            self.bind_addr,
            self.service_descriptor.full_name(),
            methods.join(",")
        )
    }
}

/// Body type that includes gRPC trailers for successful responses
struct GrpcResponseBody {
    data: Option<Bytes>,
    trailers_sent: bool,
}

impl GrpcResponseBody {
    fn new(data: Bytes) -> Self {
        Self {
            data: Some(data),
            trailers_sent: false,
        }
    }
}

impl http_body::Body for GrpcResponseBody {
    type Data = Bytes;
    type Error = Status;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
        // First send the data frame
        if let Some(data) = self.data.take() {
            return Poll::Ready(Some(Ok(http_body::Frame::data(data))));
        }

        // Then send trailers
        if !self.trailers_sent {
            self.trailers_sent = true;
            let mut trailers = http::HeaderMap::new();
            trailers.insert(
                "grpc-status",
                "0".parse()
                    .unwrap_or_else(|_| http::HeaderValue::from_static("0")),
            );
            trailers.insert(
                "grpc-message",
                "".parse()
                    .unwrap_or_else(|_| http::HeaderValue::from_static("")),
            );
            return Poll::Ready(Some(Ok(http_body::Frame::trailers(trailers))));
        }

        // Stream is complete
        Poll::Ready(None)
    }
}

/// Body type for gRPC error responses with trailers
struct GrpcErrorBody {
    trailers_sent: bool,
    status_code: tonic::Code,
    status_message: String,
}

impl GrpcErrorBody {
    fn new(code: tonic::Code, message: &str) -> Self {
        Self {
            trailers_sent: false,
            status_code: code,
            status_message: message.to_string(),
        }
    }
}

impl http_body::Body for GrpcErrorBody {
    type Data = Bytes;
    type Error = Status;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
        // Send trailers immediately for errors (no data frame)
        if !self.trailers_sent {
            self.trailers_sent = true;
            let mut trailers = http::HeaderMap::new();
            trailers.insert(
                "grpc-status",
                (self.status_code as i32)
                    .to_string()
                    .parse()
                    .unwrap_or_else(|_| http::HeaderValue::from_static("13")),
            );
            trailers.insert(
                "grpc-message",
                self.status_message
                    .parse()
                    .unwrap_or_else(|_| http::HeaderValue::from_static("internal error")),
            );
            return Poll::Ready(Some(Ok(http_body::Frame::trailers(trailers))));
        }

        // Stream is complete
        Poll::Ready(None)
    }
}

/// Multi-method dynamic gRPC service implementation
/// This handles multiple methods on a single gRPC server
struct MultiMethodGrpcService {
    method_handlers: Arc<
        RwLock<
            std::collections::HashMap<
                String,
                Arc<dyn Fn(DynamicMessage) -> Result<DynamicMessage> + Send + Sync>,
            >,
        >,
    >,
    service_descriptor: ServiceDescriptor,
}

impl MultiMethodGrpcService {
    /// Convert into a tonic service
    fn into_service(self) -> MultiMethodServiceWrapper {
        MultiMethodServiceWrapper {
            inner: Arc::new(self),
        }
    }

    /// Handle a gRPC request for a specific method
    async fn handle_request(
        &self,
        method_name: &str,
        request_bytes: Bytes,
    ) -> std::result::Result<Bytes, Status> {
        // Get the method descriptor
        let method = self
            .service_descriptor
            .methods()
            .find(|m| m.name() == method_name)
            .ok_or_else(|| Status::not_found(format!("Method {method_name} not found")))?;

        let input_descriptor = method.input();
        let _output_descriptor = method.output();

        println!(
            "  Method: {}, input descriptor: {}",
            method_name,
            input_descriptor.full_name()
        );
        println!("  Input fields:");
        for field in input_descriptor.fields() {
            println!(
                "    - {} (field {}, kind: {:?})",
                field.name(),
                field.number(),
                field.kind()
            );
        }

        // Get the handler for this method
        let handler = {
            let handlers = self.method_handlers.read().await;
            println!("  Looking up handler for method: {method_name}");
            println!(
                "  Available handlers: {:?}",
                handlers.keys().collect::<Vec<_>>()
            );
            handlers.get(method_name).cloned().ok_or_else(|| {
                Status::unimplemented(format!("No handler for method {method_name}"))
            })?
        };

        println!(
            "  About to decode {} bytes into {}",
            request_bytes.len(),
            input_descriptor.full_name()
        );
        // Decode request bytes into DynamicMessage
        let request_msg =
            DynamicMessage::decode(input_descriptor.clone(), request_bytes).map_err(|e| {
                eprintln!("  Decode error: {e}");
                Status::invalid_argument(format!("Failed to decode request: {e}"))
            })?;
        println!("  Successfully decoded request");

        // Call the handler
        let response_msg =
            (handler)(request_msg).map_err(|e| Status::internal(format!("Handler error: {e}")))?;

        // Encode response
        let mut response_bytes = Vec::new();
        response_msg
            .encode(&mut response_bytes)
            .map_err(|e| Status::internal(format!("Failed to encode response: {e}")))?;

        Ok(Bytes::from(response_bytes))
    }
}

/// Wrapper to make ``MultiMethodGrpcService`` compatible with tonic's service infrastructure
#[derive(Clone)]
struct MultiMethodServiceWrapper {
    inner: Arc<MultiMethodGrpcService>,
}

impl TowerService<http::Request<BoxBody>> for MultiMethodServiceWrapper {
    type Response = http::Response<BoxBody>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>>
                + Send,
        >,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        println!("  MultiMethodServiceWrapper::poll_ready called");
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<BoxBody>) -> Self::Future {
        println!("  MultiMethodServiceWrapper::call invoked!");
        let inner = self.inner.clone();
        let service_name = inner.service_descriptor.full_name().to_string();

        Box::pin(async move {
            // Parse path to extract service and method name: /{service}/{method}
            let path = req.uri().path().to_string();

            println!("  gRPC request path: {path}");

            // Extract method name from path (format: /package.Service/Method)
            let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
            if parts.len() != 2 {
                println!("  Invalid gRPC path format - returning 404");
                let body = Full::new(Bytes::new())
                    .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                let boxed = BoxBody::new(body);
                let response = http::Response::builder()
                    .status(404)
                    .body(boxed)
                    .unwrap_or_else(|_| {
                        let body = Full::new(Bytes::new())
                            .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                        let boxed = BoxBody::new(body);
                        http::Response::new(boxed)
                    });
                return Ok(response);
            }

            let request_service_name = match parts.first() {
                Some(name) => *name,
                None => {
                    println!("  Missing service name in path - returning 400");
                    let body = Full::new(Bytes::new())
                        .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                    let boxed = BoxBody::new(body);
                    let response = http::Response::builder()
                        .status(400)
                        .body(boxed)
                        .unwrap_or_else(|_| {
                            let body =
                                Full::new(Bytes::new()).map_err(|_: std::convert::Infallible| {
                                    Status::internal("unreachable")
                                });
                            let boxed = BoxBody::new(body);
                            http::Response::new(boxed)
                        });
                    return Ok(response);
                }
            };
            let method_name = match parts.get(1) {
                Some(name) => *name,
                None => {
                    println!("  Missing method name in path - returning 400");
                    let body = Full::new(Bytes::new())
                        .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                    let boxed = BoxBody::new(body);
                    let response = http::Response::builder()
                        .status(400)
                        .body(boxed)
                        .unwrap_or_else(|_| {
                            let body =
                                Full::new(Bytes::new()).map_err(|_: std::convert::Infallible| {
                                    Status::internal("unreachable")
                                });
                            let boxed = BoxBody::new(body);
                            http::Response::new(boxed)
                        });
                    return Ok(response);
                }
            };

            println!("  Request service: {request_service_name}, method: {method_name}");
            println!("  Our service descriptor: {service_name}");

            // Check if this request is for our service
            if *request_service_name != service_name {
                println!("  Service name mismatch - returning 404");
                let body = Full::new(Bytes::new())
                    .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                let boxed = BoxBody::new(body);
                let response = http::Response::builder()
                    .status(404)
                    .body(boxed)
                    .unwrap_or_else(|_| {
                        let body = Full::new(Bytes::new())
                            .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                        let boxed = BoxBody::new(body);
                        http::Response::new(boxed)
                    });
                return Ok(response);
            }

            // Extract request body
            let (_parts, body) = req.into_parts();

            // Read the body bytes
            let body_bytes = match body.collect().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    let body = Full::new(Bytes::new())
                        .map_err(|_: std::convert::Infallible| Status::internal("unreachable"));
                    let boxed = BoxBody::new(body);
                    let response = http::Response::builder()
                        .status(500)
                        .header("content-type", "application/grpc")
                        .header("grpc-status", "13") // INTERNAL error code
                        .header("grpc-message", "Failed to read request body")
                        .body(boxed)
                        .unwrap_or_else(|_| {
                            let body =
                                Full::new(Bytes::new()).map_err(|_: std::convert::Infallible| {
                                    Status::internal("unreachable")
                                });
                            let boxed = BoxBody::new(body);
                            http::Response::new(boxed)
                        });
                    return Ok(response);
                }
            };
            let mut request_bytes = body_bytes.to_bytes();

            println!("  Raw body length: {}", request_bytes.len());
            if let Some(first_bytes) = request_bytes.get(0..5) {
                println!("  First 5 bytes: {:?}", first_bytes);
            }

            // gRPC uses a 5-byte frame: [compressed flag (1 byte)][message length (4 bytes)][message]
            // Skip the 5-byte gRPC frame header
            if request_bytes.len() >= 5 {
                request_bytes = request_bytes.slice(5..);
                println!(
                    "  After skipping frame header, message length: {}",
                    request_bytes.len()
                );
                if !request_bytes.is_empty() {
                    println!("  Message bytes: {:?}", &request_bytes[..]);
                }
            }

            // Handle the request
            match inner.handle_request(method_name, request_bytes).await {
                Ok(response_bytes) => {
                    // gRPC requires a 5-byte frame header: [compressed flag (1 byte)][message length (4 bytes)]
                    // Add the frame header to the response
                    let mut framed_response = Vec::with_capacity(5 + response_bytes.len());
                    framed_response.push(0); // No compression
                    framed_response.extend_from_slice(&(response_bytes.len() as u32).to_be_bytes());
                    framed_response.extend_from_slice(&response_bytes);

                    // Create body with trailers support
                    let body = GrpcResponseBody::new(Bytes::from(framed_response));
                    let boxed = BoxBody::new(body);

                    let response = http::Response::builder()
                        .status(200)
                        .header("content-type", "application/grpc")
                        .body(boxed)
                        .unwrap_or_else(|_| {
                            let body = GrpcResponseBody::new(Bytes::new());
                            let boxed = BoxBody::new(body);
                            http::Response::new(boxed)
                        });
                    Ok(response)
                }
                Err(status) => {
                    // Create error body with trailers
                    let body = GrpcErrorBody::new(status.code(), status.message());
                    let boxed = BoxBody::new(body);

                    let response = http::Response::builder()
                        .status(200)
                        .header("content-type", "application/grpc")
                        .body(boxed)
                        .unwrap_or_else(|_| {
                            let body = GrpcErrorBody::new(
                                tonic::Code::Internal,
                                "Failed to build response",
                            );
                            let boxed = BoxBody::new(body);
                            http::Response::new(boxed)
                        });
                    Ok(response)
                }
            }
        })
    }
}

// Implement NamedService to satisfy tonic's add_service() requirement
// We use an empty string because our custom call() method handles all routing dynamically
impl NamedService for MultiMethodServiceWrapper {
    const NAME: &'static str = "";
}

/// Combined service that routes between reflection and dynamic gRPC handler
#[derive(Clone)]
struct CombinedService<R> {
    reflection: R,
    dynamic_router: MultiMethodServiceWrapper,
}

impl<R> TowerService<http::Request<BoxBody>> for CombinedService<R>
where
    R: TowerService<
            http::Request<BoxBody>,
            Response = http::Response<BoxBody>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    R::Future: Send + 'static,
{
    type Response = http::Response<BoxBody>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>>
                + Send,
        >,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<BoxBody>) -> Self::Future {
        let path = req.uri().path().to_string();
        let mut reflection = self.reflection.clone();
        let mut router = self.dynamic_router.clone();

        Box::pin(async move {
            // Route reflection requests to reflection service
            // gRPC reflection uses paths like:
            // - /grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo
            // - /grpc.reflection.v1.ServerReflection/ServerReflectionInfo
            if path.contains("grpc.reflection") || path.contains("ServerReflection") {
                println!("  Routing to reflection service: {path}");
                reflection.call(req).await
            } else {
                // All other requests go to our dynamic handler
                println!("  Routing to dynamic handler: {path}");
                router.call(req).await
            }
        })
    }
}

impl<R> NamedService for CombinedService<R> {
    const NAME: &'static str = "";
}

/// Adapter to convert reflection service's Body type to BoxBody
#[derive(Clone)]
struct ReflectionAdapter<S> {
    inner: S,
}

impl<S, B> TowerService<http::Request<BoxBody>> for ReflectionAdapter<S>
where
    S: TowerService<
            http::Request<BoxBody>,
            Response = http::Response<B>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    B: http_body::Body + Send + 'static,
    B::Data: bytes::Buf + Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    type Response = http::Response<BoxBody>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>>
                + Send,
        >,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<BoxBody>) -> Self::Future {
        let future = self.inner.call(req);

        Box::pin(async move {
            let response = future.await?;
            // Convert the body type from tonic::body::Body to BoxBody by mapping frames
            let (parts, body) = response.into_parts();

            // Use map_frame to convert frame-by-frame without collecting
            use http_body_util::BodyExt as _;
            let mapped_body = body
                .map_frame(|frame| {
                    frame.map_data(|mut data| {
                        // Convert Buf to Bytes
                        data.copy_to_bytes(data.remaining())
                    })
                })
                .map_err(|e| Status::from_error(e.into()));

            let boxed_body = BoxBody::new(mapped_body);
            Ok(http::Response::from_parts(parts, boxed_body))
        })
    }
}

/// Adapter to convert between hyper's Incoming body and tonic's BoxBody
#[derive(Clone)]
struct HyperAdapter<S> {
    inner: S,
}

impl<S> HyperService<http::Request<hyper::body::Incoming>> for HyperAdapter<S>
where
    S: TowerService<
            http::Request<BoxBody>,
            Response = http::Response<BoxBody>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = http::Response<BoxBody>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<Self::Response, Self::Error>>
                + Send,
        >,
    >;

    fn call(&self, req: http::Request<hyper::body::Incoming>) -> Self::Future {
        // Convert hyper::body::Incoming to BoxBody
        let (parts, body) = req.into_parts();
        let boxed_body = BoxBody::new(body.map_err(|e| Status::internal(e.to_string())));
        let req = http::Request::from_parts(parts, boxed_body);

        // Clone inner service and call it
        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}
