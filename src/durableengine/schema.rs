//! Workflow-level input and output schema validation.
//!
//! A workflow may declare contracts for the data it accepts
//! (`input.schema.document`) and the data it produces
//! (`output.schema.document`) as inline JSON Schemas. Declaring them is
//! optional; once declared they are enforced — input is validated before
//! execution starts, and output is validated (after the workflow's
//! `output.as` transform) before the workflow is marked completed. Every
//! violation is reported at once, with JSON-pointer paths.

use serverless_workflow_core::models::{schema::SchemaDefinition, workflow::WorkflowDefinition};

use super::{Error, Result};

/// Validate `input` against the workflow's declared input schema, if any.
pub(crate) fn validate_workflow_input(
    workflow: &WorkflowDefinition,
    input: &serde_json::Value,
) -> Result<()> {
    match workflow.input.as_ref().and_then(|i| i.schema.as_ref()) {
        Some(schema) => validate_against(schema, input, "input"),
        None => Ok(()),
    }
}

/// Validate `output` against the workflow's declared output schema, if any.
///
/// Per the spec, the schema describes the workflow's *transformed* output —
/// call this after applying `output.as`.
pub(crate) fn validate_workflow_output(
    workflow: &WorkflowDefinition,
    output: &serde_json::Value,
) -> Result<()> {
    match workflow.output.as_ref().and_then(|o| o.schema.as_ref()) {
        Some(schema) => validate_against(schema, output, "output"),
        None => Ok(()),
    }
}

/// Validate `value` against an inline JSON Schema definition.
///
/// Only inline `format: json` schema documents are supported: other formats
/// and external schema resources produce an error rather than silently
/// passing, so a declared contract is never quietly ignored.
fn validate_against(
    schema: &SchemaDefinition,
    value: &serde_json::Value,
    direction: &str,
) -> Result<()> {
    // The format may carry a version suffix ("json:draft-07"); match the base.
    let format_base = schema.format.split(':').next().unwrap_or("json");
    if format_base != "json" {
        return Err(Error::WorkflowExecution {
            message: format!(
                "Unsupported {direction} schema format '{}': only 'json' schemas are enforced",
                schema.format
            ),
        });
    }

    let Some(document) = schema.document.as_ref() else {
        return Err(Error::WorkflowExecution {
            message: format!(
                "{direction} schema declares no inline document; external schema resources \
                 are not supported"
            ),
        });
    };

    check_document(document, value, direction)
        .map_err(|message| Error::WorkflowExecution { message })
}

/// Validate `value` against an inline JSON Schema `document`.
///
/// Returns the full violation list (JSON-pointer paths included) as the error
/// message. Shared by workflow execution and interactive sessions so both
/// report contract violations identically.
pub(crate) fn check_document(
    document: &serde_json::Value,
    value: &serde_json::Value,
    label: &str,
) -> std::result::Result<(), String> {
    let validator =
        jsonschema::validator_for(document).map_err(|e| format!("Invalid {label} schema: {e}"))?;

    let violations: Vec<String> = validator
        .iter_errors(value)
        .map(|err| {
            let path = err.instance_path.to_string();
            let at = if path.is_empty() {
                "(root)".to_string()
            } else {
                path
            };
            format!("  - {at}: {err}")
        })
        .collect();

    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Workflow {label} failed schema validation:\n{}",
            violations.join("\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    fn workflow(extra_block: &str) -> WorkflowDefinition {
        let yaml = format!(
            r"
document:
  dsl: '1.0.2'
  namespace: test
  name: schema-test
  version: '0.1.0'
{extra_block}
do:
  - noop:
      set: '${{ . }}'
"
        );
        serde_yaml::from_str(&yaml).unwrap()
    }

    const INPUT_SCHEMA_BLOCK: &str = r"
input:
  schema:
    format: json
    document:
      type: object
      required: [working_dir, port]
      properties:
        working_dir: {type: string}
        port: {type: integer}
";

    const OUTPUT_SCHEMA_BLOCK: &str = r"
output:
  schema:
    format: json
    document:
      type: object
      required: [scoped_pbf_file]
      properties:
        scoped_pbf_file: {type: string}
";

    #[test]
    fn workflows_without_schemas_pass() {
        let wf = workflow("");
        assert!(validate_workflow_input(&wf, &json!({"anything": true})).is_ok());
        assert!(validate_workflow_output(&wf, &json!({"anything": true})).is_ok());
    }

    #[test]
    fn conforming_input_passes() {
        let wf = workflow(INPUT_SCHEMA_BLOCK);
        let input = json!({"working_dir": "/tmp", "port": 8080, "extra": "ok"});
        assert!(validate_workflow_input(&wf, &input).is_ok());
    }

    #[test]
    fn missing_required_input_key_fails_with_details() {
        let wf = workflow(INPUT_SCHEMA_BLOCK);
        let err = validate_workflow_input(&wf, &json!({"port": 8080})).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("working_dir"),
            "message should name the missing key: {msg}"
        );
    }

    #[test]
    fn wrong_input_type_fails_with_path() {
        let wf = workflow(INPUT_SCHEMA_BLOCK);
        let input = json!({"working_dir": "/tmp", "port": "not-a-number"});
        let err = validate_workflow_input(&wf, &input).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("/port"),
            "message should point at the bad field: {msg}"
        );
    }

    #[test]
    fn all_violations_reported_at_once() {
        let wf = workflow(INPUT_SCHEMA_BLOCK);
        let err = validate_workflow_input(&wf, &json!({"port": "nope"})).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("working_dir") && msg.contains("/port"),
            "{msg}"
        );
    }

    #[test]
    fn conforming_output_passes() {
        let wf = workflow(OUTPUT_SCHEMA_BLOCK);
        let output = json!({"scoped_pbf_file": "/tmp/scoped.osm.pbf", "extra": 1});
        assert!(validate_workflow_output(&wf, &output).is_ok());
    }

    #[test]
    fn nonconforming_output_fails() {
        let wf = workflow(OUTPUT_SCHEMA_BLOCK);
        let err = validate_workflow_output(&wf, &json!({})).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("output") && msg.contains("scoped_pbf_file"),
            "{msg}"
        );
    }

    #[test]
    fn non_json_format_is_rejected() {
        let wf = workflow(
            r"
input:
  schema:
    format: avro
    document: {}
",
        );
        let err = validate_workflow_input(&wf, &json!({})).unwrap_err();
        assert!(err.to_string().contains("avro"));
    }

    #[test]
    fn schema_without_document_is_rejected() {
        let wf = workflow(
            r"
input:
  schema:
    format: json
",
        );
        let err = validate_workflow_input(&wf, &json!({})).unwrap_err();
        assert!(err.to_string().contains("no inline document"));
    }

    #[test]
    fn nested_refs_resolve_without_remote_fetching() {
        // pydantic-style schema with $defs/$ref must validate self-contained.
        let wf = workflow(
            r"
input:
  schema:
    format: json
    document:
      type: object
      required: [bbox]
      properties:
        bbox: {'$ref': '#/$defs/Bbox'}
      $defs:
        Bbox:
          type: object
          required: [xmin, ymin]
          properties:
            xmin: {type: number}
            ymin: {type: number}
",
        );
        let ok = json!({"bbox": {"xmin": -77.1, "ymin": 38.8}});
        assert!(validate_workflow_input(&wf, &ok).is_ok());

        let bad = json!({"bbox": {"xmin": "x"}});
        let err = validate_workflow_input(&wf, &bad).unwrap_err();
        assert!(err.to_string().contains("/bbox"));
    }
}
