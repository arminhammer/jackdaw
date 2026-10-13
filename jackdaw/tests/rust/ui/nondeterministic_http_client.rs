// I/O belongs in a task. Expected error: move the request into a `#[task]` and `ctx.call` it.
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

#[workflow(namespace = "ui", name = "http-client", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        let _body = reqwest::get("https://example.com").await;
        Ok(ctx.call(Greet, &input).await?)
    }
}

fn main() {}
