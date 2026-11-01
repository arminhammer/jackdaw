// A spawned task escapes the journal. Expected error: put the work in a `#[task]` and `ctx.call` it.
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

#[workflow(namespace = "ui", name = "tokio-spawn", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        tokio::spawn(async { 1 + 1 });
        Ok(ctx.call(Greet, &input).await?)
    }
}

fn main() {}
