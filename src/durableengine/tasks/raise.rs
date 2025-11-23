use crate::context::Context;

use super::super::{DurableEngine, Error, Result};

/// Execute a Raise task - raises an error with structured error information
pub async fn exec_raise_task(
    _engine: &DurableEngine,
    task_name: &str,
    raise_task: &serverless_workflow_core::models::task::RaiseTaskDefinition,
    _ctx: &Context,
) -> Result<serde_json::Value> {
    use serverless_workflow_core::models::error::OneOfErrorDefinitionOrReference;

    // Extract the error definition
    let error_def = match &raise_task.raise.error {
        OneOfErrorDefinitionOrReference::Error(err) => err,
        OneOfErrorDefinitionOrReference::Reference(ref_name) => {
            return Err(Error::Configuration { message: format!("Error references not yet implemented: {}", ref_name) });
        }
    };

    // Build the error object according to the spec
    let mut error_obj = serde_json::json!({
        "type": error_def.type_,
        "title": error_def.title,
        "status": error_def.status,
    });

    // Add optional fields if present
    if let Some(detail) = &error_def.detail {
        error_obj.as_object_mut().unwrap().insert(
            "detail".to_string(),
            serde_json::Value::String(detail.clone()),
        );
    }

    // Add the instance field - this should be the path to the task in the workflow
    // The path format is /do/index/taskName
    let task_path = format!("/do/0/{}", task_name);
    error_obj
        .as_object_mut()
        .unwrap()
        .insert("instance".to_string(), serde_json::Value::String(task_path));

    // Serialize the error to a JSON string for the error message
    let error_json = serde_json::to_string(&error_obj)?;

    // Return an error with the JSON-serialized error object
    Err(Error::TaskExecution { message: error_json })
}