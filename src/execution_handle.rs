//! Execution handle for controlling and observing workflow execution
//!
//! The [`ExecutionHandle`] provides control over a running workflow execution,
//! allowing you to stream events, wait for completion, or cancel the workflow.

use crate::workflow::WorkflowEvent;
use snafu::prelude::*;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Workflow execution error: {message}"))]
    WorkflowExecution { message: String },

    #[snafu(display("Timeout: {message}"))]
    Timeout { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Handle for controlling and observing a workflow execution
///
/// This handle provides three main capabilities:
/// 1. Stream events from the workflow execution
/// 2. Wait for the workflow to complete (one-shot workflows)
/// 3. Cancel the workflow (essential for perpetual workflows)
///
/// # Examples
///
/// ## One-shot workflow - wait for result
/// ```no_run
/// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource};
/// # use std::time::Duration;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = DurableEngineBuilder::new().build()?;
/// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
/// let mut handle = engine.execute(source, serde_json::json!({})).await?;
/// let result = handle.wait_for_completion(Duration::from_secs(60)).await?;
/// println!("Result: {}", result);
/// # Ok(())
/// # }
/// ```
///
/// ## Stream events from execution
/// ```no_run
/// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource, workflow::WorkflowEvent};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = DurableEngineBuilder::new().build()?;
/// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
/// let mut handle = engine.execute(source, serde_json::json!({})).await?;
///
/// while let Some(event) = handle.next_event().await {
///     match event {
///         WorkflowEvent::WorkflowCorrelationCompleted { correlation_output, correlation_context, .. } => {
///             if let Some(output) = correlation_output {
///                 println!("Correlation {} result: {}", correlation_context, output);
///             }
///         }
///         WorkflowEvent::WorkflowCompleted { .. } => {
///             println!("Workflow finished");
///             break;
///         }
///         _ => {}
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// ## Perpetual workflow - process indefinitely
/// ```no_run
/// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource, workflow::WorkflowEvent};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let engine = DurableEngineBuilder::new().build()?;
/// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
/// let mut handle = engine.execute(source, serde_json::json!({})).await?;
///
/// // Process events until shutdown signal
/// while let Some(event) = handle.next_event().await {
///     if let WorkflowEvent::WorkflowCorrelationCompleted { correlation_output, correlation_context, .. } = event {
///         if let Some(output) = correlation_output {
///             println!("Request {} result: {}", correlation_context, output);
///         }
///     }
/// }
///
/// handle.cancel().await?;
/// # Ok(())
/// # }
/// ```
pub struct ExecutionHandle {
    instance_id: String,
    event_receiver: mpsc::Receiver<WorkflowEvent>,
    cancel_sender: mpsc::Sender<()>,
}

impl ExecutionHandle {
    /// Create a new execution handle
    ///
    /// This is typically called by the engine, not by users directly.
    #[must_use]
    pub(crate) fn new(
        instance_id: String,
        event_receiver: mpsc::Receiver<WorkflowEvent>,
        cancel_sender: mpsc::Sender<()>,
    ) -> Self {
        Self {
            instance_id,
            event_receiver,
            cancel_sender,
        }
    }

    /// Get the instance ID of the executing workflow
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = DurableEngineBuilder::new().build()?;
    /// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
    /// let handle = engine.execute(source, serde_json::json!({})).await?;
    /// println!("Workflow instance: {}", handle.instance_id());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Get the next event from the execution stream
    ///
    /// Returns `None` when:
    /// - The workflow completes (one-shot workflows)
    /// - The workflow is cancelled
    /// - The workflow fails
    /// - The event stream is closed
    ///
    /// For perpetual workflows, this continues indefinitely until the workflow
    /// is cancelled or fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource, workflow::WorkflowEvent};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = DurableEngineBuilder::new().build()?;
    /// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
    /// let mut handle = engine.execute(source, serde_json::json!({})).await?;
    ///
    /// while let Some(event) = handle.next_event().await {
    ///     println!("Event: {:?}", event);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn next_event(&mut self) -> Option<WorkflowEvent> {
        self.event_receiver.recv().await
    }

    /// Cancel the workflow execution
    ///
    /// This sends a cancellation signal to the workflow. The cancellation is graceful -
    /// the workflow will stop after the current task completes.
    ///
    /// This method is essential for stopping perpetual workflows that run indefinitely.
    ///
    /// # Errors
    /// Returns an error if the cancellation signal could not be sent (e.g., if the
    /// workflow has already stopped).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = DurableEngineBuilder::new().build()?;
    /// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
    /// let handle = engine.execute(source, serde_json::json!({})).await?;
    ///
    /// // ... later ...
    /// handle.cancel().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cancel(self) -> Result<()> {
        self.cancel_sender
            .send(())
            .await
            .map_err(|_| Error::WorkflowExecution {
                message: "Failed to send cancellation signal".into(),
            })
    }

    /// Wait for the workflow to complete and return the final result
    ///
    /// This method waits for a `WorkflowCompleted` event and returns the final data.
    ///
    /// **Important**: This method is only suitable for one-shot workflows.
    /// For perpetual workflows (e.g., with listeners that never complete),
    /// this will timeout as they never emit `WorkflowCompleted`.
    ///
    /// Use [`next_event()`](Self::next_event) for perpetual workflows instead.
    ///
    /// # Errors
    /// Returns an error if:
    /// - The workflow fails
    /// - The workflow is cancelled
    /// - The timeout is exceeded
    /// - The event stream closes unexpectedly
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jackdaw::{DurableEngineBuilder, workflow_source::StringSource};
    /// # use std::time::Duration;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let engine = DurableEngineBuilder::new().build()?;
    /// # let source = StringSource::new("document:\n  dsl: '1.0.0-alpha1'\n  namespace: test\n  name: test\ndo: []");
    /// let mut handle = engine.execute(source, serde_json::json!({})).await?;
    /// let result = handle.wait_for_completion(Duration::from_secs(60)).await?;
    /// println!("Result: {}", result);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_completion(mut self, timeout: Duration) -> Result<serde_json::Value> {
        let start = std::time::Instant::now();

        while let Some(event) = self.next_event().await {
            match event {
                WorkflowEvent::WorkflowCompleted { final_data, .. } => {
                    return Ok(final_data);
                }
                WorkflowEvent::WorkflowFailed { error, .. } => {
                    return Err(Error::WorkflowExecution {
                        message: error,
                    });
                }
                WorkflowEvent::WorkflowCancelled { reason, .. } => {
                    return Err(Error::WorkflowExecution {
                        message: format!("Workflow cancelled: {}", reason.unwrap_or_default()),
                    });
                }
                _ => {}
            }

            if start.elapsed() > timeout {
                return Err(Error::Timeout {
                    message: format!("Workflow execution timed out after {timeout:?}"),
                });
            }
        }

        Err(Error::WorkflowExecution {
            message: "Event stream closed unexpectedly".into(),
        })
    }
}
