use crate::container::{ContainerConfig, ContainerProvider, ContainerResult, Error, Result};
use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    AttachContainerOptions, Config, RemoveContainerOptions, StartContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding};
use futures::StreamExt;
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;

/// Docker container provider using bollard
#[derive(Debug, Clone)]
pub struct DockerProvider {
    /// Docker client
    docker: Docker,
}

impl DockerProvider {
    /// Create a new Docker provider
    ///
    /// # Errors
    ///
    /// Returns an error if unable to connect to Docker daemon
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults().map_err(|e| Error::Provider {
            message: format!("Failed to connect to Docker daemon: {e}"),
        })?;

        Ok(Self { docker })
    }

    /// Create a new Docker provider with a custom Docker client
    #[must_use]
    #[allow(dead_code)]
    pub fn with_docker(docker: Docker) -> Self {
        Self { docker }
    }
}

#[async_trait]
impl ContainerProvider for DockerProvider {
    async fn execute(&self, config: ContainerConfig) -> Result<ContainerResult> {
        // Prepare command - if command is empty, use default shell
        let cmd = if config.command.is_empty() {
            vec!["/bin/sh".to_string(), "-c".to_string()]
        } else {
            config.command
        };

        // Prepare environment variables
        let env: Option<Vec<String>> = config
            .environment
            .as_ref()
            .map(|env_map| env_map.iter().map(|(k, v)| format!("{k}={v}")).collect());

        // Prepare volumes (bind mounts) with SELinux relabeling for compatibility
        // Using binds with :z suffix for proper SELinux support on Fedora/RHEL/CentOS
        let binds: Option<Vec<String>> = config.volumes.as_ref().map(|vols| {
            vols.iter()
                .map(|(host_path, container_path)| {
                    // Add :z for SELinux relabeling (shared volume label)
                    // This allows the container to write to the host directory
                    format!("{}:{}:z", host_path, container_path)
                })
                .collect()
        });

        // Prepare port bindings
        let (exposed_ports, port_bindings) = if let Some(ports) = config.ports.as_ref() {
            let mut exposed = HashMap::new();
            let mut bindings = HashMap::new();

            for (container_port, host_port) in ports {
                // Exposed ports format: "8080/tcp" -> {}
                let port_key = format!("{container_port}/tcp");
                exposed.insert(port_key.clone(), HashMap::new());

                // Port bindings format: "8080/tcp" -> [{"HostPort": "8080"}]
                bindings.insert(
                    port_key,
                    Some(vec![PortBinding {
                        host_ip: None,
                        host_port: Some(host_port.to_string()),
                    }]),
                );
            }

            (Some(exposed), Some(bindings))
        } else {
            (None, None)
        };

        // Create host configuration for volumes and ports
        let host_config = if binds.is_some() || port_bindings.is_some() {
            Some(HostConfig {
                binds,
                port_bindings,
                ..Default::default()
            })
        } else {
            None
        };

        let image_parts: Vec<&str> = config.image.split(':').collect();
        let (image_name, image_tag) = match (image_parts.first(), image_parts.get(1)) {
            (Some(&name), Some(&tag)) => (name, tag),
            (Some(&name), None) => (name, "latest"),
            _ => (config.image.as_str(), "latest"),
        };

        let create_image_options = CreateImageOptions {
            from_image: image_name,
            tag: image_tag,
            ..Default::default()
        };

        let mut pull_stream = self
            .docker
            .create_image(Some(create_image_options), None, None);

        // Process pull stream (this will pull the image if not present, or be a no-op if cached)
        while let Some(pull_result) = pull_stream.next().await {
            pull_result.map_err(|e| Error::ImagePull {
                message: format!("Failed to pull image {}: {}", config.image, e),
            })?;
        }

        // Create container configuration
        let container_config = Config {
            image: Some(config.image.clone()),
            cmd: Some(cmd),
            env,
            working_dir: config.working_dir.clone(),
            attach_stdin: Some(config.stdin.is_some()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            open_stdin: Some(config.stdin.is_some()),
            stdin_once: Some(config.stdin.is_some()),
            tty: Some(false),
            exposed_ports,
            host_config,
            ..Default::default()
        };

        // Create container
        let container = self
            .docker
            .create_container::<String, String>(None, container_config)
            .await
            .map_err(|e| Error::Creation {
                message: format!("Failed to create container: {e}"),
            })?;

        let container_id = container.id.clone();

        // Attach to container before starting it
        let attach_options = AttachContainerOptions::<String> {
            stdin: Some(config.stdin.is_some()),
            stdout: Some(true),
            stderr: Some(true),
            stream: Some(true),
            logs: Some(false),
            detach_keys: None,
        };

        let attach_result = self
            .docker
            .attach_container(&container_id, Some(attach_options))
            .await
            .map_err(|e| Error::Attach {
                message: format!("Failed to attach to container: {e}"),
            })?;

        // Start container
        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| Error::Start {
                message: format!("Failed to start container: {e}"),
            })?;

        // Write stdin if provided
        let bollard::container::AttachContainerResults {
            mut output,
            mut input,
        } = attach_result;

        if let Some(stdin_str) = config.stdin.as_ref() {
            input
                .write_all(stdin_str.as_bytes())
                .await
                .map_err(|e| Error::Io {
                    message: format!("Failed to write stdin: {e}"),
                })?;
            input.shutdown().await.map_err(|e| Error::Io {
                message: format!("Failed to close stdin: {e}"),
            })?;
        }

        // Collect output
        let mut stdout_buffer = String::new();
        let mut stderr_buffer = String::new();

        while let Some(output_result) = output.next().await {
            let output_chunk = output_result.map_err(|e| Error::Io {
                message: format!("Failed to read output: {e}"),
            })?;

            match output_chunk {
                bollard::container::LogOutput::StdOut { message } => {
                    stdout_buffer.push_str(&String::from_utf8_lossy(&message));
                }
                bollard::container::LogOutput::StdErr { message } => {
                    stderr_buffer.push_str(&String::from_utf8_lossy(&message));
                }
                bollard::container::LogOutput::StdIn { .. } => {}
                bollard::container::LogOutput::Console { message } => {
                    stdout_buffer.push_str(&String::from_utf8_lossy(&message));
                }
            }
        }

        // Get exit code from container inspection
        let inspect = self
            .docker
            .inspect_container(&container_id, None)
            .await
            .map_err(|e| Error::Inspect {
                message: format!("Failed to inspect container: {e}"),
            })?;

        let exit_code = inspect.state.and_then(|s| s.exit_code).unwrap_or(0);

        // Remove container
        let remove_options = RemoveContainerOptions {
            force: true,
            ..Default::default()
        };

        self.docker
            .remove_container(&container_id, Some(remove_options))
            .await
            .map_err(|e| Error::Execution {
                message: format!("Failed to remove container: {e}"),
            })?;

        Ok(ContainerResult {
            stdout: stdout_buffer,
            stderr: stderr_buffer,
            exit_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_docker_execute_simple() {
        let provider = DockerProvider::new();

        // Skip test if Docker is not available
        if provider.is_err() {
            eprintln!("Skipping test: Docker not available");
            return;
        }

        let provider = provider.unwrap();

        let config = ContainerConfig {
            image: "alpine".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            stdin: None,
            environment: None,
            working_dir: None,
            volumes: None,
            ports: None,
        };

        let result = provider.execute(config).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello world"));
    }

    #[tokio::test]
    async fn test_docker_execute_with_stdin() {
        let provider = DockerProvider::new();

        // Skip test if Docker is not available
        if provider.is_err() {
            eprintln!("Skipping test: Docker not available");
            return;
        }

        let provider = provider.unwrap();

        let config = ContainerConfig {
            image: "alpine".to_string(),
            command: vec!["/bin/sh".to_string(), "-c".to_string(), "cat".to_string()],
            stdin: Some("test input".to_string()),
            environment: None,
            working_dir: None,
            volumes: None,
            ports: None,
        };

        let result = provider.execute(config).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("test input"));
    }

    #[tokio::test]
    async fn test_docker_execute_with_env() {
        let provider = DockerProvider::new();

        // Skip test if Docker is not available
        if provider.is_err() {
            eprintln!("Skipping test: Docker not available");
            return;
        }

        let provider = provider.unwrap();

        let mut env = std::collections::HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let config = ContainerConfig {
            image: "alpine".to_string(),
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo $TEST_VAR".to_string(),
            ],
            stdin: None,
            environment: Some(env),
            working_dir: None,
            volumes: None,
            ports: None,
        };

        let result = provider.execute(config).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("test_value"));
    }
}
