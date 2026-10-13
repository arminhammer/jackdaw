//! Event streaming example
//!
//! Run with: cargo run --example event_streaming

use jackdaw::DurableEngineBuilder;
use serverless_workflow_core::models::{
    task::{SetTaskDefinition, SetValue, TaskDefinition},
    workflow::{DocumentMetadata, WorkflowDefinition},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = DurableEngineBuilder::new().build()?;

    let workflow = WorkflowDefinition {
        document: DocumentMetadata {
            dsl: "1.0.2".to_string(),
            namespace: "examples".to_string(),
            name: "streaming-example".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        },
        do_: vec![
            (
                "step1".to_string(),
                TaskDefinition::Set(SetTaskDefinition {
                    set: SetValue::Inline(serde_json::json!({ "count": 1 })),
                    ..Default::default()
                }),
            ),
            (
                "step2".to_string(),
                TaskDefinition::Set(SetTaskDefinition {
                    set: SetValue::Inline(serde_json::json!({ "count": "${ .count + 1 }" })),
                    ..Default::default()
                }),
            ),
            (
                "step3".to_string(),
                TaskDefinition::Set(SetTaskDefinition {
                    set: SetValue::Inline(serde_json::json!({ "count": "${ .count + 1 }" })),
                    ..Default::default()
                }),
            ),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };

    let mut handle = engine.execute(workflow, serde_json::json!({})).await?;

    while let Some(event) = handle.next_event().await {
        match event {
            jackdaw::workflow::WorkflowEvent::WorkflowStarted { instance_id, .. } => {
                println!("Started: {}", instance_id);
            }
            jackdaw::workflow::WorkflowEvent::TaskStarted { task_name, .. } => {
                println!("Task started: {}", task_name);
            }
            jackdaw::workflow::WorkflowEvent::TaskCompleted {
                task_name, result, ..
            } => {
                println!("Task completed: {} -> {}", task_name, result);
            }
            jackdaw::workflow::WorkflowEvent::WorkflowCompleted { final_data, .. } => {
                println!("Completed: {}", serde_json::to_string_pretty(&final_data)?);
                break;
            }
            jackdaw::workflow::WorkflowEvent::WorkflowFailed { error, .. } => {
                eprintln!("Failed: {}", error);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
