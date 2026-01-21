//! Basic workflow execution example
//!
//! Run with: cargo run --example simple_workflow

use jackdaw::DurableEngineBuilder;
use serverless_workflow_core::models::{
    task::{SetTaskDefinition, SetValue, TaskDefinition},
    workflow::{DocumentMetadata, WorkflowDefinition},
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = DurableEngineBuilder::new().build()?;

    let workflow = WorkflowDefinition {
        document: DocumentMetadata {
            dsl: "1.0.2".to_string(),
            namespace: "examples".to_string(),
            name: "simple-transform".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        },
        do_: vec![
            (
                "greet".to_string(),
                TaskDefinition::Set(SetTaskDefinition {
                    set: SetValue::Inline(serde_json::json!({
                        "greeting": "Hello, World!",
                        "timestamp": "${ now() }"
                    })),
                    ..Default::default()
                }),
            ),
            (
                "transform".to_string(),
                TaskDefinition::Set(SetTaskDefinition {
                    set: SetValue::Inline(serde_json::json!({
                        "message": "${ \"Greeting: \" + .greeting }",
                        "time": "${ .timestamp }"
                    })),
                    ..Default::default()
                }),
            ),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };

    let input = serde_json::json!({ "user": "Alice" });
    let handle = engine.execute(workflow, input).await?;
    let result = handle.wait_for_completion(Duration::from_secs(30)).await?;

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
