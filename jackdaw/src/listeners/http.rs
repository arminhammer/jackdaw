use super::{Listener, Result};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, RawPathParams},
    http::StatusCode,
    response::IntoResponse,
    routing::{MethodRouter, any},
};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Create a method router that handles all HTTP methods (GET, POST, PUT, DELETE, PATCH, etc.)
///
/// The handler gets the request's JSON body with the route's path parameters and query string added
/// (see [`super::request_input`]).
fn create_method_router(
    handler: Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
) -> MethodRouter {
    any(
        move |params: RawPathParams, Query(query): Query<BTreeMap<String, String>>, body: Bytes| {
            let handler = handler.clone();
            async move {
                let payload = match super::request_input(&params, query, &body) {
                    Ok(payload) => payload,
                    Err(message) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": message })),
                        )
                            .into_response();
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
                        )
                            .into_response()
                    }
                }
            }
        },
    )
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
                let method_router = create_method_router(handler_clone);

                app = app.route(&path_str, method_router);
            }
        }

        // Parse bind address
        let addr: std::net::SocketAddr = bind_addr.parse().map_err(|e| super::Error::Listener {
            message: format!("Invalid bind address {bind_addr}: {e}"),
        })?;

        // Bound before the listener counts as started, so a port that cannot be had fails the workflow.
        let listener = super::bind(addr).await?;

        // Spawn server in background
        let server_handle = crate::api::observe::spawn(async move {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn path_and_query_parameters_use_the_route_names() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let echo: Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync> =
            Arc::new(Ok);
        let listener = HttpListener::new_multi_route(
            format!("127.0.0.1:{port}"),
            [("/orders/{orderId}/items/{item}".to_string(), echo)].into(),
        )
        .unwrap();
        listener.start().await.unwrap();

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/orders/42/items/book?verbose=true");
        let get: serde_json::Value = client.get(&url).send().await.unwrap().json().await.unwrap();
        assert_eq!(
            get,
            serde_json::json!({ "orderId": 42, "item": "book", "verbose": true })
        );
        let post: serde_json::Value = client
            .post(&url)
            .json(&serde_json::json!({ "item": "pen", "count": 2 }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            post,
            serde_json::json!({ "orderId": 42, "item": "pen", "count": 2, "verbose": true }),
            "the body wins over a parameter of the same name"
        );
        let bad = client.post(&url).body("not json").send().await.unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        listener.stop().await.unwrap();
    }
}
