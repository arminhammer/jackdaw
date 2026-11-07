//! Rust and YAML on one engine: Rust runs an Open Workflow YAML file, and the YAML calls a Rust task by name.
//!
//! ```text
//! cargo run -p jackdaw-wf --example yaml_interop
//! ```
use jackdaw::config::Layers;
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};

/// The workflow's input, read from `GREET_` environment variables.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Config)]
pub struct Name {
    /// Who to greet
    #[serde(default = "ada")]
    pub name: String,
}

fn ada() -> String {
    "Ada".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Greeting {
    pub greeting: String,
}

#[task(name = "greet")]
async fn greet(input: Name) -> Result<Greeting, TaskError> {
    Ok(Greeting {
        greeting: format!("Hello, {}!", input.name),
    })
}

const CALLS_GREET: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: calls-greet, version: '1.0.0' }
do:
  - hello:
      call: greet
      with:
        name: '${ .name }'
"#;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let input: Name = Layers::new().env("GREET_").load()?;
    let input_name = input.name.clone();
    let engine = Engine::builder().task(Greet).build()?;
    // The YAML workflow is bound to Rust types; its input and output are checked against them.
    let workflow = Workflow::<Name, Greeting>::from_yaml(CALLS_GREET)?;

    let greeting = engine.execute(&workflow, input.clone()).await?.await?;

    println!("{}", greeting.greeting);
    assert_eq!(greeting.greeting, format!("Hello, {}!", input_name));
    Ok(())
}
