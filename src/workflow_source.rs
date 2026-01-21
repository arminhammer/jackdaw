//! Workflow loading from various sources
//!
//! This module provides the [`WorkflowSource`] trait and implementations for loading
//! workflow definitions from different sources like files, strings, or bytes.

use async_trait::async_trait;
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::path::{Path, PathBuf};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Failed to load workflow from {location}: {message}"))]
    LoadFailed { location: String, message: String },

    #[snafu(display("Failed to parse workflow"))]
    ParseFailed {
        #[snafu(source)]
        source: serde_yaml::Error,
    },

    #[snafu(display("I/O error"))]
    Io {
        #[snafu(source)]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Trait for loading workflow definitions from various sources
///
/// This trait allows workflows to be loaded from different sources such as:
/// - Filesystem ([`FilesystemSource`])
/// - String literals ([`StringSource`])
/// - Raw bytes ([`BytesSource`])
/// - Custom sources (implement this trait)
///
/// # Examples
///
/// ```
/// use jackdaw::workflow_source::{WorkflowSource, StringSource};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let source = StringSource::new(r#"
/// document:
///   dsl: '1.0.0-alpha1'
///   namespace: example
///   name: hello
/// do:
///   - greet: { set: { message: "Hello!" } }
/// "#);
///
/// let workflow = source.load().await?;
/// assert_eq!(workflow.document.name, "hello");
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait WorkflowSource: Send + Sync {
    /// Load and parse a workflow definition
    ///
    /// # Errors
    /// Returns an error if the workflow cannot be loaded or parsed
    async fn load(&self) -> Result<WorkflowDefinition>;

    /// Get a human-readable identifier for this source
    ///
    /// Used primarily for error messages
    fn source_identifier(&self) -> String;
}

/// Load workflow from a filesystem path
///
/// # Examples
///
/// ```no_run
/// use jackdaw::workflow_source::{WorkflowSource, FilesystemSource};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let source = FilesystemSource::new("./workflows/example.yaml");
/// let workflow = source.load().await?;
/// # Ok(())
/// # }
/// ```
pub struct FilesystemSource {
    path: PathBuf,
}

impl FilesystemSource {
    /// Create a new filesystem source
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

#[async_trait]
impl WorkflowSource for FilesystemSource {
    async fn load(&self) -> Result<WorkflowDefinition> {
        let content = tokio::fs::read_to_string(&self.path)
            .await
            .context(IoSnafu)?;

        serde_yaml::from_str(&content).context(ParseFailedSnafu)
    }

    fn source_identifier(&self) -> String {
        self.path.display().to_string()
    }
}

/// Load workflow from a YAML or JSON string
///
/// # Examples
///
/// ```
/// use jackdaw::workflow_source::{WorkflowSource, StringSource};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let yaml = r#"
/// document:
///   dsl: '1.0.0-alpha1'
///   namespace: test
///   name: example
/// do: []
/// "#;
///
/// let source = StringSource::new(yaml);
/// let workflow = source.load().await?;
/// # Ok(())
/// # }
/// ```
pub struct StringSource {
    content: String,
}

impl StringSource {
    /// Create a new string source from YAML or JSON content
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[async_trait]
impl WorkflowSource for StringSource {
    async fn load(&self) -> Result<WorkflowDefinition> {
        serde_yaml::from_str(&self.content).context(ParseFailedSnafu)
    }

    fn source_identifier(&self) -> String {
        "<string>".to_string()
    }
}

/// Load workflow from raw bytes (YAML or JSON)
///
/// This is useful when you've already read the workflow data from a custom source
/// and need to parse it.
///
/// # Examples
///
/// ```
/// use jackdaw::workflow_source::{WorkflowSource, BytesSource};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let yaml_bytes = b"document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: example\ndo: []";
///
/// let source = BytesSource::new(yaml_bytes.to_vec());
/// let workflow = source.load().await?;
/// # Ok(())
/// # }
/// ```
pub struct BytesSource {
    content: Vec<u8>,
}

impl BytesSource {
    /// Create a new bytes source
    #[must_use]
    pub fn new(content: Vec<u8>) -> Self {
        Self { content }
    }
}

#[async_trait]
impl WorkflowSource for BytesSource {
    async fn load(&self) -> Result<WorkflowDefinition> {
        let content = std::str::from_utf8(&self.content).map_err(|e| Error::LoadFailed {
            location: self.source_identifier(),
            message: format!("Invalid UTF-8: {e}"),
        })?;

        serde_yaml::from_str(content).context(ParseFailedSnafu)
    }

    fn source_identifier(&self) -> String {
        "<bytes>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_WORKFLOW: &str = r#"
document:
  dsl: '1.0.0-alpha1'
  namespace: test
  name: example
  version: '1.0.0'
do:
  - testTask:
      set:
        value: 123
"#;

    #[tokio::test]
    async fn test_string_source_valid_yaml() {
        let source = StringSource::new(VALID_WORKFLOW);
        let workflow = source.load().await.unwrap();
        assert_eq!(workflow.document.name, "example");
        assert_eq!(workflow.document.namespace, "test");
    }

    #[tokio::test]
    async fn test_string_source_invalid_yaml() {
        let source = StringSource::new("invalid: yaml: [");
        let result = source.load().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bytes_source_valid() {
        let source = BytesSource::new(VALID_WORKFLOW.as_bytes().to_vec());
        let workflow = source.load().await.unwrap();
        assert_eq!(workflow.document.name, "example");
    }

    #[tokio::test]
    async fn test_bytes_source_invalid_utf8() {
        let invalid_utf8 = vec![0xFF, 0xFE, 0xFD];
        let source = BytesSource::new(invalid_utf8);
        let result = source.load().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid UTF-8"));
    }

    #[tokio::test]
    async fn test_source_identifier() {
        let string_source = StringSource::new(VALID_WORKFLOW);
        assert_eq!(string_source.source_identifier(), "<string>");

        let bytes_source = BytesSource::new(vec![]);
        assert_eq!(bytes_source.source_identifier(), "<bytes>");
    }
}
