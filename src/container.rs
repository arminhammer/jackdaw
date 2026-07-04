use async_trait::async_trait;
use snafu::prelude::*;

/// Container execution result
#[derive(Debug, Clone)]
pub struct ContainerResult {
    /// Standard output from the container
    pub stdout: String,
    /// Standard error from the container
    pub stderr: String,
    /// Exit code
    pub exit_code: i64,
}

/// Container execution configuration
#[derive(Debug, Clone, Default)]
pub struct ContainerConfig {
    /// Container image name
    pub image: String,
    /// Explicit container name (passed as --name to Docker); auto-assigned if None
    pub name: Option<String>,
    /// Command to execute
    pub command: Vec<String>,
    /// Standard input to provide to the container
    pub stdin: Option<String>,
    /// Environment variables
    pub environment: Option<std::collections::HashMap<String, String>>,
    /// Working directory
    pub working_dir: Option<String>,
    /// Volume mappings (host_path -> container_path)
    pub volumes: Option<std::collections::HashMap<String, String>>,
    /// Port mappings (container_port -> host_port)
    pub ports: Option<std::collections::HashMap<u16, u16>>,
    /// Task name used for labeling streamed output lines
    pub task_name: String,
    /// Task index used for color selection in streamed output
    pub task_index: usize,
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("Container provider error: {message}"))]
    Provider { message: String },

    #[snafu(display("Container creation failed: {message}"))]
    Creation { message: String },

    #[snafu(display("Container start failed: {message}"))]
    Start { message: String },

    #[snafu(display("Container attach failed: {message}"))]
    Attach { message: String },

    #[snafu(display("Container execution failed: {message}"))]
    Execution { message: String },

    #[snafu(display("Container I/O error: {message}"))]
    Io { message: String },

    #[snafu(display("Container wait failed: {message}"))]
    Wait { message: String },

    #[snafu(display("Container inspect failed: {message}"))]
    Inspect { message: String },

    #[snafu(display("Image pull failed: {message}"))]
    ImagePull { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Container provider trait for executing containers
#[async_trait]
pub trait ContainerProvider: Send + Sync + std::fmt::Debug {
    /// Execute a container with the given configuration
    ///
    /// # Errors
    ///
    /// Returns an error if container creation, execution, or cleanup fails
    async fn execute(&self, config: ContainerConfig) -> Result<ContainerResult>;

    /// Stop a running container by name
    ///
    /// # Errors
    ///
    /// Returns an error if the container cannot be found or stopped
    #[allow(dead_code)]
    async fn stop_container(&self, name: &str) -> Result<()>;
}
