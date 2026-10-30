//! Call types of your own: YAML workflows `call:` them by name, like the built-in `http` or `grpc`.
use super::error::TaskError;
use serde_json::Value;

/// One `call:` of a custom call type, as the executor sees it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Call {
    /// The call type, the name the executor was registered under (`kafka`).
    pub call_type: String,
    /// The name of the task in the workflow that made the call.
    pub task_name: String,
    /// The task's `with:` arguments, expressions evaluated.
    pub arguments: Value,
    /// The instance that made the call.
    pub instance_id: String,
}

/// A call type of your own, registered with [`EngineBuilder::executor`](crate::api::EngineBuilder::executor).
///
/// ```
/// use jackdaw::api::{Call, Executor};
/// use jackdaw::prelude::*;
/// use serde_json::{Value, json};
///
/// struct Kafka;
///
/// #[async_trait::async_trait]
/// impl Executor for Kafka {
///     async fn exec(&self, call: Call) -> Result<Value, TaskError> {
///         // Publish `call.arguments` ...
///         Ok(json!({ "published": call.arguments["topic"] }))
///     }
/// }
///
/// let engine = Engine::builder().executor("kafka", Kafka).build()?;
/// # Ok::<(), Error>(())
/// ```
#[async_trait::async_trait]
pub trait Executor: Send + Sync + 'static {
    /// Performs the call and returns the task's output. A [`TaskError`] fails the task (and is retried by its retry
    /// policy when it is retryable).
    async fn exec(&self, call: Call) -> Result<Value, TaskError>;

    /// Whether results may be reused for a call with the same arguments. Calls are side effects, so the default is no.
    fn cacheable(&self) -> bool {
        false
    }
}

/// A shared executor: register an `Arc` and keep a clone to look at its state.
#[async_trait::async_trait]
impl<T: Executor + ?Sized> Executor for std::sync::Arc<T> {
    async fn exec(&self, call: Call) -> Result<Value, TaskError> {
        (**self).exec(call).await
    }

    fn cacheable(&self) -> bool {
        (**self).cacheable()
    }
}
