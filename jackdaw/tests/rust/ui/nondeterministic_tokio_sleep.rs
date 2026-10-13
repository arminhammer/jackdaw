// A real sleep is not durable. Expected error: use `ctx.sleep(duration)`.
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Name {
    pub name: String,
}

#[task]
async fn greet(input: Name) -> Result<String, TaskError> {
    Ok(input.name)
}

#[workflow(namespace = "ui", name = "tokio-sleep", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        Ok(ctx.call(Greet, &input).await?)
    }
}

fn main() {}
