use crate::context::Context;
use async_trait::async_trait;
use serde_json;
use snafu::prelude::*;

#[derive(Debug, Snafu)]
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
    ) -> Result<serde_json::Value>;

    /// Downcast to concrete type for special handling
    fn as_any(&self) -> &dyn std::any::Any;
}
