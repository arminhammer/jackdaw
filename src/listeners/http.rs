use super::{Listener, Result};
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::Request,
    http::StatusCode,
    response::IntoResponse,
    routing::{any, MethodRouter},
    body::Body,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Create a method router that handles all HTTP methods (GET, POST, PUT, DELETE, PATCH, etc.)
///
/// For requests with bodies (POST, PUT, PATCH), extracts JSON payload.
/// For requests without bodies (GET, DELETE), extracts path parameters.
fn create_method_router(
    handler: Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
) -> MethodRouter {
    any(move |request: Request<Body>| {
        let handler = handler.clone();
        async move {
            // Extract the request parts
            let (parts, body) = request.into_parts();
            let method = parts.method.clone();

            // Build the payload based on HTTP method
            let payload = if method == axum::http::Method::GET || method == axum::http::Method::DELETE {
                // For GET/DELETE, extract path parameters from the URI
                // The path params are in the URI path segments after the base path
                let uri_path = parts.uri.path();

                // Extract path parameters from the URI
                // For example, /api/v1/pet/123 -> {"petId": "123"}
                let mut params = serde_json::Map::new();

                // Try to parse path segments
                // This is a simplified approach - in a real implementation,
                // we'd need the route pattern to properly extract params
                let segments: Vec<&str> = uri_path.split('/').filter(|s| !s.is_empty()).collect();

                // For now, if the last segment is numeric, assume it's an ID parameter
                // More sophisticated parameter extraction would require route pattern matching
                if let Some(last_segment) = segments.last() {
                    // Try to parse as a number
                    if let Ok(num) = last_segment.parse::<i64>() {
                        params.insert("petId".to_string(), serde_json::json!(num));
                    } else {
                        params.insert("petId".to_string(), serde_json::json!(last_segment));
                    }
                }

                serde_json::Value::Object(params)
            } else {
                // For POST/PUT/PATCH, extract JSON body
                let bytes = match axum::body::to_bytes(body, usize::MAX).await {
                    Ok(b) => b,
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": format!("Failed to read request body: {}", e)
                            })),
                        ).into_response();
                    }
                };

                if bytes.is_empty() {
                    serde_json::json!({})
                } else {
                    match serde_json::from_slice(&bytes) {
                        Ok(json) => json,
                        Err(e) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({
                                    "error": format!("Invalid JSON: {}", e)
                                })),
                            ).into_response();
                        }
                    }
                }
            };

            // Call the handler with the payload
            match handler(payload) {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(e) => {
                    tracing::error!("Handler error: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": e.to_string()
                        })),
                    ).into_response()
                }
            }
        }
    })
}

/// HTTP/OpenAPI listener for handling REST requests
pub struct HttpListener {
    /// Bind address (e.g., "localhost:8080")
    bind_addr: String,

    /// Route handlers: path -> handler function
    /// For multi-route servers, this contains all handlers
    /// Using ``RwLock`` to allow adding routes dynamically
    route_handlers: Arc<
        RwLock<
            std::collections::HashMap<
                String,
                Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
            >,
        >,
    >,

    /// Server handle for shutdown
    shutdown_tx: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,

    /// Server task handle
    server_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl HttpListener {
    // /// Add a route handler to an existing listener
    // /// This allows adding new routes to an already-running server
    // pub async fn add_route(
    //     &self,
    //     path: String,
    //     handler: Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
    // ) -> Result<()> {
    //     // Add to handlers map
    //     let mut handlers = self.route_handlers.write().await;
    //     handlers.insert(path.clone(), handler);

    //     tracing::info!(
    //         "Added route {} to HTTP listener on {}",
    //         path,
    //         self.bind_addr
    //     );
    //     Ok(())
    // }

    /// Create a new HTTP listener with multiple route handlers
    /// This allows a single server to handle multiple paths on the same port
    ///
    /// # Errors
    /// This function currently does not return an error and will always succeed; it returns `Ok(Self)`.
    pub fn new_multi_route(
        bind_addr: String,
        route_handlers: std::collections::HashMap<
            String,
            Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
        >,
    ) -> Result<Self> {
        Ok(Self {
            bind_addr,
            route_handlers: Arc::new(RwLock::new(route_handlers)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
        })
    }
}

#[async_trait]
impl Listener for HttpListener {
    async fn start(&self) -> Result<()> {
        let routes: Vec<String> = {
            let handlers = self.route_handlers.read().await;
            handlers.keys().cloned().collect()
        };
        tracing::info!(
            "Starting HTTP listener on {} for paths {:?}",
            self.bind_addr,
            routes
        );

        let bind_addr = self.bind_addr.clone();
        let route_handlers = self.route_handlers.clone();

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Store shutdown sender
        {
            let mut tx_lock = self.shutdown_tx.write().await;
            *tx_lock = Some(shutdown_tx);
        }

        // Create the axum app with dynamic routing
        let mut app = Router::new();

        // Add all routes to the router with individual handlers
        {
            let handlers = route_handlers.read().await;
            for (path, handler) in handlers.iter() {
                let handler_clone = handler.clone();
                let path_str = path.clone();

                // Create a unified handler that supports all HTTP methods
                // For GET/DELETE: extract path params and use empty body
                // For POST/PUT/PATCH: use JSON body
                let method_router = create_method_router(handler_clone);

                app = app.route(&path_str, method_router);
            }
        }

        // Parse bind address
        let addr: std::net::SocketAddr = bind_addr.parse().map_err(|e| super::Error::Listener {
            message: format!("Invalid bind address {bind_addr}: {e}"),
        })?;

        // Spawn server in background
        let server_handle = tokio::spawn(async move {
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind to {}: {}", addr, e);
                    return;
                }
            };

            tracing::info!("HTTP server listening on {}", addr);

            // Convert the stateless router into a make_service
            if let Err(e) = axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
            {
                tracing::error!("Server error: {}", e);
            }
        });

        // Store server handle
        {
            let mut handle_lock = self.server_handle.write().await;
            *handle_lock = Some(server_handle);
        }

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        tracing::info!("Stopping HTTP listener on {}", self.bind_addr);

        // Send shutdown signal
        {
            let mut shutdown = self.shutdown_tx.write().await;
            if let Some(tx) = shutdown.take() {
                let _ = tx.send(());
            }
        }

        // Wait for server to finish
        {
            let mut handle_lock = self.server_handle.write().await;
            if let Some(handle) = handle_lock.take() {
                let _ = handle.await;
            }
        }

        Ok(())
    }

    fn get_endpoint(&self) -> String {
        // Note: This is synchronous but we need to read from RwLock
        // We'll use blocking read which is OK for this use case
        let routes: Vec<String> = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let handlers = self.route_handlers.read().await;
                handlers.keys().cloned().collect()
            })
        });
        format!("http://{}/[{}]", self.bind_addr, routes.join(","))
    }
}

impl std::fmt::Debug for HttpListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpListener")
            .field("bind_addr", &self.bind_addr)
            .field("openapi_spec", &"<OpenAPI spec>")
            .field("route_handlers", &"<function handlers>")
            .field("shutdown_tx", &"<shutdown sender>")
            .field("server_handle", &"<server task>")
            .finish()
    }
}
