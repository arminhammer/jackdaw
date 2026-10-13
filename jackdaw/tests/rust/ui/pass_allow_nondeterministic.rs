// Audited code can opt out of the determinism lint, per item.
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

#[workflow(namespace = "ui", name = "opt-out", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<String> {
        #[allow_nondeterminism]
        let _audited = std::time::SystemTime::now();
        Ok(ctx.call(Greet, &input).await?)
    }
}

fn main() {}
