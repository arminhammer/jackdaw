use async_trait::async_trait;
#[cfg(feature = "yaml")]
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use std::sync::Arc;

#[cfg(feature = "grpc-listener")]
pub mod grpc;
// The YAML interpreter's HTTP server (the engine's own listeners serve with axum directly, in `api::listener`).
#[cfg(all(feature = "yaml", feature = "http-listener"))]
pub mod http;

// pub use grpc::GrpcListener;
#[cfg(all(feature = "yaml", feature = "http-listener"))]
pub use http::HttpListener;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Listener error: {message}"))]
    Listener { message: String },

    #[snafu(display("Server error: {message}"))]
    Server { message: String },

    #[snafu(display("Execution error: {message}"))]
    Execution { message: String },
}

// From implementations for automatic error conversion
impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Listener {
            message: format!("IO error: {source}"),
        }
    }
}

impl From<serde_saphyr::Error> for Error {
    fn from(source: serde_saphyr::Error) -> Self {
        Error::Listener {
            message: format!("YAML error: {source}"),
        }
    }
}

#[cfg(feature = "grpc-listener")]
impl From<protox::Error> for Error {
    fn from(source: protox::Error) -> Self {
        Error::Listener {
            message: format!("Proto compilation error: {source}"),
        }
    }
}

#[cfg(feature = "grpc-listener")]
impl From<prost::EncodeError> for Error {
    fn from(source: prost::EncodeError) -> Self {
        Error::Listener {
            message: format!("Proto encoding error: {source}"),
        }
    }
}

#[cfg(feature = "grpc-listener")]
impl From<prost_reflect::DescriptorError> for Error {
    fn from(source: prost_reflect::DescriptorError) -> Self {
        Error::Listener {
            message: format!("Proto descriptor error: {source}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Binds `addr` for a listener. A listener whose engine just stopped releases its port as it shuts down, so a port in
/// use is retried for a moment before giving up.
// Used by the YAML interpreter's listeners (the engine binds its own synchronously).
#[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
#[cfg_attr(not(feature = "yaml"), allow(dead_code))]
pub(crate) async fn bind(addr: std::net::SocketAddr) -> Result<tokio::net::TcpListener> {
    let mut attempts = 0;
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempts < 50 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => {
                return Err(Error::Listener {
                    message: format!("cannot listen on {addr}: {e}"),
                });
            }
        }
    }
}

/// Listener trait for handling incoming events from various sources
#[async_trait]
pub trait Listener: Send + Sync {
    /// Start the listener and begin accepting requests
    async fn start(&self) -> Result<()>;

    /// Stop the listener and clean up resources
    async fn stop(&self) -> Result<()>;

    /// Get the endpoint this listener is bound to
    fn get_endpoint(&self) -> String;
}

#[cfg(feature = "yaml")]
/// Event source configuration from workflow Listen task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSource {
    /// URI of the event source (e.g., <grpc://localhost:50051/service.Method> or <http://localhost:8080/path>)
    pub uri: String,

    /// Schema definition for the event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaReference>,
}

#[cfg(feature = "yaml")]
/// Schema reference for event validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaReference {
    /// Schema format (proto, openapi, etc.)
    pub format: String,

    /// Resource location
    pub resource: ResourceLocation,
}

#[cfg(feature = "yaml")]
/// Resource location for schema files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLocation {
    /// Path to the schema file
    pub endpoint: String,

    /// Optional name/reference within the schema (e.g., message name in ``OpenAPI``)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Registry for managing multiple listeners
#[derive(Default)]
pub struct ListenerRegistry {
    listeners: Vec<Arc<dyn Listener>>,
}

#[allow(dead_code)]
impl ListenerRegistry {
    /// Create a new listener registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            listeners: Vec::new(),
        }
    }

    /// Register a listener
    pub fn register(&mut self, listener: Arc<dyn Listener>) {
        self.listeners.push(listener);
    }

    /// Start all registered listeners
    /// # Errors
    /// Returns an error if any listener fails to start.
    pub async fn start_all(&self) -> Result<()> {
        for listener in &self.listeners {
            listener.start().await?;
        }
        Ok(())
    }

    /// Stop all registered listeners
    /// # Errors
    /// Returns an error if any listener fails to stop.
    pub async fn stop_all(&self) -> Result<()> {
        for listener in &self.listeners {
            listener.stop().await?;
        }
        Ok(())
    }

    /// Get all listener endpoints
    #[must_use]
    pub fn get_endpoints(&self) -> Vec<String> {
        self.listeners.iter().map(|l| l.get_endpoint()).collect()
    }
}
