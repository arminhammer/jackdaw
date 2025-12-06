use chrono::Utc;

use crate::context::Context;

use super::super::{DurableEngine, Result};

/// Execute an Emit task - emits ``CloudEvents`` to the workflow context
pub async fn exec_emit_task(
    _engine: &DurableEngine,
    _task_name: &str,
    emit_task: &serverless_workflow_core::models::task::EmitTaskDefinition,
    ctx: &Context,
) -> Result<serde_json::Value> {
    // Get current context data for expression evaluation
    let current_data = ctx.state.data.read().await.clone();

    // Evaluate the event attributes
    let mut event_data = serde_json::Map::new();

    // CloudEvents standard fields
    // Generate a unique ID for the event
    event_data.insert(
        "id".to_string(),
        serde_json::json!(uuid::Uuid::new_v4().to_string()),
    );

    // CloudEvents spec version
    event_data.insert("specversion".to_string(), serde_json::json!("1.0"));

    // Add timestamp
    event_data.insert(
        "time".to_string(),
        serde_json::json!(Utc::now().to_rfc3339()),
    );

    // Process the 'with' attributes from the event definition
    for (key, value) in &emit_task.emit.event.with {
        let evaluated_value = crate::expressions::evaluate_value_with_input(
            value,
            &current_data,
            &ctx.metadata.initial_input,
        )?;
        event_data.insert(key.clone(), evaluated_value);
    }

    let result = serde_json::Value::Object(event_data);

    // Merge each field of the event into the context (not nested under task name)
    if let serde_json::Value::Object(map) = &result {
        for (key, value) in map {
            ctx.merge(key, value.clone()).await;
        }
    }

    Ok(result)
}
