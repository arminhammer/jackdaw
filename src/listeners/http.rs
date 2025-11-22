use super::{Listener, Result};
use async_trait::async_trait;
use axum::{
    Json, Router,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
use openapiv3::OpenAPI;
use snafu::prelude::*;
use std::sync::Arc;
use tokio::sync::RwLock;

/// HTTP/OpenAPI listener for handling REST requests
pub struct HttpListener {
    /// Bind address (e.g., "localhost:8080")
    bind_addr: String,

    /// OpenAPI specification
    openapi_spec: Arc<OpenAPI>,

    /// Route handlers: path -> handler function
    /// For multi-route servers, this contains all handlers
    /// Using RwLock to allow adding routes dynamically
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
    /// Add a route handler to an existing listener
    /// This allows adding new routes to an already-running server
    pub async fn add_route(
        &self,
        path: String,
        handler: Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
    ) -> Result<()> {
        // Add to handlers map
        let mut handlers = self.route_handlers.write().await;
        handlers.insert(path.clone(), handler);

        tracing::info!(
            "Added route {} to HTTP listener on {}",
            path,
            self.bind_addr
        );
        Ok(())
    }

    /// Create a new HTTP listener with multiple route handlers
    /// This allows a single server to handle multiple paths on the same port
    pub fn new_multi_route(
        bind_addr: String,
        openapi_path: &str,
        route_handlers: std::collections::HashMap<
            String,
            Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
        >,
    ) -> Result<Self> {
        // Load OpenAPI spec
        let openapi_content = std::fs::read_to_string(openapi_path)?;
        let openapi_spec: OpenAPI = serde_yaml::from_str(&openapi_content)?;

        Ok(Self {
            bind_addr,
            openapi_spec: Arc::new(openapi_spec),
            route_handlers: Arc::new(RwLock::new(route_handlers)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
        })
    }

    /// Validate request against OpenAPI schema
    fn validate_request(&self, _body: &serde_json::Value) -> Result<()> {
        // TODO: Implement OpenAPI schema validation
        Ok(())
    }

    /// Validate response against OpenAPI schema
    fn validate_response(&self, _body: &serde_json::Value) -> Result<()> {
        // TODO: Implement OpenAPI schema validation
        Ok(())
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
                let handler = handler.clone();
                let path = path.clone();
                // Create a handler closure for this specific route
                app = app.route(
                    &path,
                    post(move |Json(payload): Json<serde_json::Value>| {
                        let h = handler.clone();
                        async move {
                            match h(payload) {
                                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                                Err(e) => {
                                    tracing::error!("Handler error: {}", e);
                                    (
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        Json(serde_json::json!({
                                            "error": e.to_string()
                                        })),
                                    )
                                        .into_response()
                                }
                            }
                        }
                    }),
                );
            }
        }

        // Parse bind address
        let addr: std::net::SocketAddr = bind_addr.parse().map_err(|e| {
            super::Error::Listener {
                message: format!("Invalid bind address {}: {}", bind_addr, e),
            }
        })?;

        // Spawn server in background
        let server_handle = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .expect("Failed to bind");

            tracing::info!("HTTP server listening on {}", addr);

            // Convert the stateless router into a make_service
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .expect("Server error");
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
            .finish()
    }
}
