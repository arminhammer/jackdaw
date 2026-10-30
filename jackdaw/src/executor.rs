use crate::context::Context;
use crate::task_output::TaskOutputStreamer;
use async_trait::async_trait;
use snafu::prelude::*;

/// Why a call failed in the YAML engine's executors.
#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum Error {
    #[snafu(display("Execution error: {message}"))]
    Execution { message: String },

    #[snafu(display("Task error: {message}"))]
    Task { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

#[async_trait]
pub trait Executor: Send + Sync {
    async fn exec(
        &self,
        task_name: &str,
        params: &serde_json::Value,
        ctx: &Context,
        streamer: Option<TaskOutputStreamer>,
    ) -> Result<serde_json::Value>;

    /// Whether a `call` to this executor may be answered from the cache. Built-in protocols are, native tasks are
    /// side effects and are not (unless they opt in).
    fn cacheable(&self) -> bool {
        true
    }
}
