// `tokio::select!` races without recording the winner. Expected error: use `jackdaw::select!`.
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

#[workflow(namespace = "ui", name = "tokio-select", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        tokio::select! {
            greeting = ctx.call(Greet, &input) => Ok(greeting?),
            () = ctx.sleep(std::time::Duration::from_secs(1)) => Ok(String::new()),
        }
    }
}

fn main() {}
