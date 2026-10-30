//! A call type of your own: implement `Executor` and register it by name, and YAML workflows `call:` it like the
//! built-in `http` or `grpc`.
//!
//! ```text
//! cargo run -p jackdaw --example custom_executor
//! ```
use jackdaw::api::Call;
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, PoisonError};

/// Pretends to publish to a message broker, and remembers what it published.
#[derive(Default)]
struct Broker {
    published: Mutex<Vec<(String, Value)>>,
}

#[async_trait::async_trait]
impl jackdaw::Executor for Broker {
    async fn exec(&self, call: Call) -> Result<Value, TaskError> {
        let topic = call
            .arguments
            .get("topic")
            .and_then(Value::as_str)
            .ok_or_else(|| TaskError::non_retryable("no topic"))?;
        let message = call.arguments.get("message").cloned().unwrap_or_default();
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        published.push((topic.to_string(), message));
        Ok(json!({ "offset": published.len() - 1 }))
    }
}

const PUBLISHES: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: publishes, version: '1.0.0' }
do:
  - announce:
      call: broker
      with:
        topic: orders
        message: '${ .order }'
"#;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let broker = Arc::new(Broker::default());
    let engine = Engine::builder()
        .executor("broker", Arc::clone(&broker))
        .build()?;
    let workflow = Workflow::<Value, Value>::from_yaml(PUBLISHES)?;

    let output = engine
        .execute(&workflow, json!({ "order": "o-1" }))
        .await?
        .await?;

    println!("{output}");
    assert_eq!(output, json!({ "offset": 0 }));
    let published = broker
        .published
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(published, [("orders".to_string(), json!("o-1"))]);
    Ok(())
}
