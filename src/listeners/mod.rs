use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use std::sync::Arc;

pub mod grpc;
pub mod http;

// pub use grpc::GrpcListener;
pub use http::HttpListener;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Listener error: {message}"))]
    Listener { message: String },

    #[snafu(display("Failed to bind to {address}: {source}"))]
    BindFailed {
        address: String,
        source: std::io::Error,
    },

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

impl From<serde_yaml::Error> for Error {
    fn from(source: serde_yaml::Error) -> Self {
        Error::Listener {
            message: format!("YAML error: {source}"),
        }
    }
}

impl From<protox::Error> for Error {
    fn from(source: protox::Error) -> Self {
        Error::Listener {
            message: format!("Proto compilation error: {source}"),
        }
    }
}

impl From<prost::EncodeError> for Error {
    fn from(source: prost::EncodeError) -> Self {
        Error::Listener {
            message: format!("Proto encoding error: {source}"),
        }
    }
}

impl From<prost_reflect::DescriptorError> for Error {
    fn from(source: prost_reflect::DescriptorError) -> Self {
        Error::Listener {
            message: format!("Proto descriptor error: {source}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

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

/// Event source configuration from workflow Listen task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSource {
    /// URI of the event source (e.g., <grpc://localhost:50051/service.Method> or <http://localhost:8080/path>)
    pub uri: String,

    /// Schema definition for the event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaReference>,
}

/// Schema reference for event validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaReference {
    /// Schema format (proto, openapi, etc.)
    pub format: String,

    /// Resource location
    pub resource: ResourceLocation,
}

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
