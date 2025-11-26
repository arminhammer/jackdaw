use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Runtime descriptor as per Serverless Workflow spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    /// A human friendly name for the runtime
    pub name: String,
    /// The version of the runtime
    pub version: String,
    /// Implementation specific key-value pairs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, Value>>,
}

impl RuntimeDescriptor {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            version,
            metadata: None,
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Map<String, Value>) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// DateTime descriptor as per Serverless Workflow spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateTimeDescriptor {
    /// ISO 8601 formatted timestamp
    pub iso8601: String,
    /// Epoch time in seconds
    #[serde(rename = "epoch")]
    pub epoch: EpochDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochDescriptor {
    /// Seconds since Unix epoch
    pub seconds: i64,
    /// Milliseconds since Unix epoch (whole timestamp, not just fractional part)
    pub milliseconds: i64,
}

impl From<DateTime<Utc>> for DateTimeDescriptor {
    fn from(dt: DateTime<Utc>) -> Self {
        Self {
            iso8601: dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            epoch: EpochDescriptor {
                seconds: dt.timestamp(),
                milliseconds: dt.timestamp_millis(),
            },
        }
    }
}

/// Workflow descriptor as per Serverless Workflow spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDescriptor {
    /// Unique ID of the workflow execution
    pub id: String,
    /// The workflow's definition as a parsed object
    pub definition: Value,
    /// The workflow's raw input (before input.from expression)
    pub input: Value,
    /// The start time of the execution
    #[serde(rename = "startedAt")]
    pub started_at: DateTimeDescriptor,
}

impl WorkflowDescriptor {
    pub fn new(id: String, definition: Value, input: Value, started_at: DateTime<Utc>) -> Self {
        Self {
            id,
            definition,
            input,
            started_at: started_at.into(),
        }
    }
}

/// Task descriptor as per Serverless Workflow spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDescriptor {
    /// The task's name
    pub name: String,
    /// The task's reference (e.g., "/do/2/myTask")
    pub reference: String,
    /// The task's definition as a parsed object
    pub definition: Value,
    /// The task's raw input (before input.from expression)
    pub input: Value,
    /// The task's raw output (before output.as expression)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// The start time of the task
    #[serde(rename = "startedAt")]
    pub started_at: DateTimeDescriptor,
}

impl TaskDescriptor {
    pub fn new(
        name: String,
        reference: String,
        definition: Value,
        input: Value,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            name,
            reference,
            definition,
            input,
            output: None,
            started_at: started_at.into(),
        }
    }

    pub fn with_output(mut self, output: Value) -> Self {
        self.output = Some(output);
        self
    }
}
