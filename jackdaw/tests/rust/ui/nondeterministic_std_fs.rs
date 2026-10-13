// Reading a file in `#[run]` can return something different on replay. Expected error: use a `#[task]`.
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

#[workflow(namespace = "ui", name = "std-fs", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        let _config = std::fs::read_to_string("config.toml");
        Ok(ctx.call(Greet, &input).await?)
    }
}

fn main() {}
