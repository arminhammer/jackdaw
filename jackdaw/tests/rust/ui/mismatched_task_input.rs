// `Greet` takes a `Name`; passing a `String` must not compile.
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

#[workflow(namespace = "ui", name = "mismatched-input", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        Ok(ctx.call(Greet, input.name.clone()).await?)
    }
}

fn main() {}
