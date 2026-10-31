use std::collections::HashMap;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use crate::models::duration::*;
use crate::models::event::*;
use crate::models::error::*;
use crate::models::map::*;
use crate::models::input::*;
use crate::models::resource::*;
use crate::models::retry::*;
use crate::models::authentication::*;

use super::output::OutputDataModelDefinition;
use super::timeout::OneOfTimeoutDefinitionOrReference;

/// Enumerates all supported task types
pub struct TaskType;
impl TaskType {
    /// Gets the type of a 'call' task
    pub const CALL: &'static str = "call";
    /// Gets the type of a 'do' task
    pub const DO: &'static str = "do";
    /// Gets the type of a 'emit' task
    pub const EMIT: &'static str = "emit";
    /// Gets the type of a 'for' task
    pub const FOR: &'static str = "for";
    /// Gets the type of a 'fork' task
    pub const FORK: &'static str = "fork";
    /// Gets the type of a 'listen' task
    pub const LISTEN: &'static str = "listen";
    /// Gets the type of a 'raise' task
    pub const RAISE: &'static str = "raise";
    /// Gets the type of a 'run' task
    pub const RUN: &'static str = "run";
    /// Gets the type of a 'set' task
    pub const SET: &'static str = "set";
    /// Gets the type of a 'switch' task
    pub const SWITCH: &'static str = "switch";
    /// Gets the type of a 'try' task
    pub const TRY: &'static str = "try";
    /// Gets the type of a 'wait' task
    pub const WAIT: &'static str = "wait";
    /// Gets the type of a 'mcp' call
    pub const CALL_MCP: &'static str = "mcp";
    /// Gets the type of a 'a2a' call
    pub const CALL_A2A: &'static str = "a2a";
}

/// Enumerates all supported process types
pub struct ProcessType;
impl ProcessType {
    /// Gets the type of a 'container' process
    pub const CONTAINER: &'static str = "container";
    /// Gets the type of a 'shell' process
    pub const SCRIPT: &'static str = "script";
    /// Gets the type of a 'shell' process
    pub const SHELL: &'static str = "shell";
    /// Gets the type of a 'workflow' process
    pub const WORKFLOW: &'static str = "workflow";
}

/// Represents a value that can be any of the supported task definitions
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TaskDefinition{
    /// Variant holding the definition of a 'call' task
    Call(CallTaskDefinition),
    /// Variant holding the definition of a 'do' task
    Do(DoTaskDefinition),
    /// Variant holding the definition of an 'emit' task
    Emit(EmitTaskDefinition),
    /// Variant holding the definition of a 'for' task
    For(ForTaskDefinition),
    /// Variant holding the definition of a 'fork' task
    Fork(ForkTaskDefinition),
    /// Variant holding the definition of a 'listen' task
    Listen(ListenTaskDefinition),
    /// Variant holding the definition of a 'raise' task
    Raise(RaiseTaskDefinition),
    /// Variant holding the definition of a 'run' task
    Run(RunTaskDefinition),
    /// Variant holding the definition of a 'set' task
    Set(SetTaskDefinition),
    /// Variant holding the definition of a 'switch' task
    Switch(SwitchTaskDefinition),
    /// Variant holding the definition of a 'try' task
    Try(TryTaskDefinition),
    /// Variant holding the definition of a 'wait' task
    Wait(WaitTaskDefinition)
}

// Custom deserializer to handle For vs Do ambiguity
impl<'de> serde::Deserialize<'de> for TaskDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        // Check for 'for' field first - if present, it's a For task
        if value.get("for").is_some() {
            return ForTaskDefinition::deserialize(value)
                .map(TaskDefinition::For)
                .map_err(serde::de::Error::custom);
        }

        // Try other variants in priority order
        if value.get("call").is_some() {
            return CallTaskDefinition::deserialize(value)
                .map(TaskDefinition::Call)
                .map_err(serde::de::Error::custom);
        }

        if value.get("set").is_some() {
            return SetTaskDefinition::deserialize(value)
                .map(TaskDefinition::Set)
                .map_err(serde::de::Error::custom);
        }

        if value.get("fork").is_some() {
            return ForkTaskDefinition::deserialize(value)
                .map(TaskDefinition::Fork)
                .map_err(serde::de::Error::custom);
        }

        if value.get("run").is_some() {
            return RunTaskDefinition::deserialize(value)
                .map(TaskDefinition::Run)
                .map_err(serde::de::Error::custom);
        }

        if value.get("switch").is_some() {
            return SwitchTaskDefinition::deserialize(value)
                .map(TaskDefinition::Switch)
                .map_err(serde::de::Error::custom);
        }

        if value.get("try").is_some() {
            return TryTaskDefinition::deserialize(value)
                .map(TaskDefinition::Try)
                .map_err(serde::de::Error::custom);
        }

        if value.get("emit").is_some() {
            return EmitTaskDefinition::deserialize(value)
                .map(TaskDefinition::Emit)
                .map_err(serde::de::Error::custom);
        }

        if value.get("raise").is_some() {
            return RaiseTaskDefinition::deserialize(value)
                .map(TaskDefinition::Raise)
                .map_err(serde::de::Error::custom);
        }

        if value.get("wait").is_some() {
            return WaitTaskDefinition::deserialize(value)
                .map(TaskDefinition::Wait)
                .map_err(serde::de::Error::custom);
        }

        if value.get("listen").is_some() {
            return ListenTaskDefinition::deserialize(value)
                .map(TaskDefinition::Listen)
                .map_err(serde::de::Error::custom);
        }

        // If we get here and there's a 'do' field, it's a Do task (not a For task)
        if value.get("do").is_some() {
            return DoTaskDefinition::deserialize(value)
                .map(TaskDefinition::Do)
                .map_err(serde::de::Error::custom);
        }

        Err(serde::de::Error::custom("unknown task type"))
    }
}

/// A trait that all task definitions must implement
pub trait TaskDefinitionBase {
    /// Gets the task's type
    fn task_type(&self) -> &str;
}

/// Holds the fields common to all tasks
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskDefinitionFields{

    /// Gets/sets a runtime expression, if any, used to determine whether or not the execute the task in the current context
    #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
    pub if_: Option<String>,

    /// Gets/sets the definition, if any, of the task's input data
    #[serde(rename = "input", skip_serializing_if = "Option::is_none")]
    pub input: Option<InputDataModelDefinition>,

    /// Gets/sets the definition, if any, of the task's output data
    #[serde(rename = "output", skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputDataModelDefinition>,

    /// Gets/sets the optional configuration for exporting data within the task's context
    #[serde(rename = "export", skip_serializing_if = "Option::is_none")]
    pub export: Option<OutputDataModelDefinition>,

    /// Gets/sets the task's timeout, if any
    #[serde(rename = "timeout", skip_serializing_if = "Option::is_none")]
    pub timeout: Option<OneOfTimeoutDefinitionOrReference>,

    /// Gets/sets the flow directive to be performed upon completion of the task
    #[serde(rename = "then", skip_serializing_if = "Option::is_none")]
    pub then: Option<String>,

    /// Gets/sets a key/value mapping of additional information associated with the task
    #[serde(rename = "metadata", skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, Value>>

}
impl Default for TaskDefinitionFields{
    fn default() -> Self {
        TaskDefinitionFields::new()
    }
}
impl TaskDefinitionFields{

    /// Initializes a new TaskDefinitionFields
    pub fn new() -> Self{
        Self { 
            if_: None, 
            input: None, 
            output: None, 
            export: None, 
            timeout: None, 
            then: None, 
            metadata: None 
        }
    }

}

/// Represents the definition of a task used to call a predefined function
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CallTaskDefinition{
    /// AsyncAPI call
    AsyncApi(Box<CallAsyncApiTaskDefinition>),
    /// gRPC call
    Grpc(Box<CallGrpcTaskDefinition>),
    /// HTTP call
    Http(Box<CallHttpTaskDefinition>),
    /// OpenAPI call
    OpenApi(Box<CallOpenApiTaskDefinition>),
    /// A2A call
    A2a(Box<CallA2aTaskDefinition>),
    /// MCP call
    Mcp(Box<CallMcpTaskDefinition>),
    /// Generic function call
    Function(Box<CallFunctionTaskDefinition>)
}

// Custom deserializer for CallTaskDefinition to handle both specific and generic calls
impl<'de> serde::Deserialize<'de> for CallTaskDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let call_val = value.get("call").and_then(|v| v.as_str()).ok_or_else(|| serde::de::Error::custom("missing 'call' field"))?;

        match call_val {
            "asyncapi" => CallAsyncApiTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::AsyncApi(Box::new(v))).map_err(serde::de::Error::custom),
            "grpc" => CallGrpcTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::Grpc(Box::new(v))).map_err(serde::de::Error::custom),
            "http" => CallHttpTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::Http(Box::new(v))).map_err(serde::de::Error::custom),
            "openapi" => CallOpenApiTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::OpenApi(Box::new(v))).map_err(serde::de::Error::custom),
            "a2a" => CallA2aTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::A2a(Box::new(v))).map_err(serde::de::Error::custom),
            "mcp" => CallMcpTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::Mcp(Box::new(v))).map_err(serde::de::Error::custom),
            _ => CallFunctionTaskDefinition::deserialize(value).map(|v| CallTaskDefinition::Function(Box::new(v))).map_err(serde::de::Error::custom),
        }
    }
}

impl TaskDefinitionBase for CallTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::CALL
    }
}

/// Represents the definition of a task used to call an AsyncAPI operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallAsyncApiTaskDefinition {
    /// The call type (must be 'asyncapi')
    #[serde(rename = "call")]
    pub call: String,
    /// Arguments for the AsyncAPI call
    #[serde(rename = "with")]
    pub with: AsyncApiArguments,
    /// Common task fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields
}

impl Default for CallAsyncApiTaskDefinition {
    fn default() -> Self {
        Self { 
            call: "asyncapi".to_string(), 
            with: AsyncApiArguments::default(), 
            common: TaskDefinitionFields::default() 
        }
    }
}

/// Arguments for an AsyncAPI call
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsyncApiArguments {
    /// Document defining the operation
    #[serde(rename = "document")]
    pub document: ExternalResourceDefinition,
    /// Channel name
    #[serde(rename = "channel", skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Operation name/ID
    #[serde(rename = "operation", skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Target server
    #[serde(rename = "server", skip_serializing_if = "Option::is_none")]
    pub server: Option<AsyncApiServerDefinition>,
    /// Protocol to use
    #[serde(rename = "protocol", skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// Message to send
    #[serde(rename = "message", skip_serializing_if = "Option::is_none")]
    pub message: Option<Value>,
    /// Subscription details
    #[serde(rename = "subscription", skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Value>,
    /// Authentication policy
    #[serde(rename = "authentication", skip_serializing_if = "Option::is_none")]
    pub authentication: Option<OneOfAuthenticationPolicyDefinitionOrReference>
}

/// Configuration for AsyncAPI server
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsyncApiServerDefinition {
    /// Name of the target server
    #[serde(rename = "name")]
    pub name: String,
    /// Environment/variables
    #[serde(rename = "variables", skip_serializing_if = "Option::is_none")]
    pub variables: Option<HashMap<String, String>>,
}

/// Represents the definition of a task used to call a gRPC service
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallGrpcTaskDefinition {
    /// The call type (must be 'grpc')
    #[serde(rename = "call")]
    pub call: String,
    /// Arguments for the gRPC call
    #[serde(rename = "with")]
    pub with: GrpcArguments,
    /// Common task fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields
}

impl Default for CallGrpcTaskDefinition {
    fn default() -> Self {
        Self { 
            call: "grpc".to_string(), 
            with: GrpcArguments::default(), 
            common: TaskDefinitionFields::default() 
        }
    }
}

/// Arguments for a gRPC call
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrpcArguments {
    /// Proto resource
    #[serde(rename = "proto")]
    pub proto: ExternalResourceDefinition,
    /// Service definition
    #[serde(rename = "service")]
    pub service: GrpcServiceDefinition,
    /// Method name
    #[serde(rename = "method")]
    pub method: String,
    /// Arguments
    #[serde(rename = "arguments", skip_serializing_if = "Option::is_none")]
    pub arguments: Option<HashMap<String, Value>>
}

/// Definition of a gRPC service
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrpcServiceDefinition {
    /// Service name
    #[serde(rename = "name")]
    pub name: String,
    /// Service host
    #[serde(rename = "host")]
    pub host: String,
    /// Service port
    #[serde(rename = "port", skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Authentication policy
    #[serde(rename = "authentication", skip_serializing_if = "Option::is_none")]
    pub authentication: Option<OneOfAuthenticationPolicyDefinitionOrReference>
}

/// Represents the definition of a task used to perform an HTTP request
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallHttpTaskDefinition {
    /// The call type (must be 'http')
    #[serde(rename = "call")]
    pub call: String,
    /// Arguments for the HTTP call
    #[serde(rename = "with")]
    pub with: HttpArguments,
    /// Common task fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields
}

impl Default for CallHttpTaskDefinition {
    fn default() -> Self {
        Self { 
            call: "http".to_string(), 
            with: HttpArguments::default(), 
            common: TaskDefinitionFields::default() 
        }
    }
}

/// Represents a name/value string mapping, or a runtime expression that evaluates to one
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOfStringMapOrRuntimeExpression {
    /// A name/value mapping
    Map(HashMap<String, String>),
    /// A runtime expression
    RuntimeExpression(String),
}

/// Arguments for an HTTP call
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpArguments {
    /// HTTP method
    #[serde(rename = "method")]
    pub method: String,
    /// Target endpoint
    #[serde(rename = "endpoint")]
    pub endpoint: OneOfEndpointDefinitionOrUri,
    /// Request headers
    #[serde(rename = "headers", skip_serializing_if = "Option::is_none")]
    pub headers: Option<OneOfStringMapOrRuntimeExpression>,
    /// Request body
    #[serde(rename = "body", skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// Query parameters
    #[serde(rename = "query", skip_serializing_if = "Option::is_none")]
    pub query: Option<OneOfStringMapOrRuntimeExpression>,
    /// Desired output format
    #[serde(rename = "output", skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Redirection strategy
    #[serde(rename = "redirect", skip_serializing_if = "Option::is_none")]
    pub redirect: Option<bool>
}

/// Represents the definition of a task used to call an OpenAPI operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallOpenApiTaskDefinition {
    /// The call type (must be 'openapi')
    #[serde(rename = "call")]
    pub call: String,
    /// Arguments for the OpenAPI call
    #[serde(rename = "with")]
    pub with: OpenApiArguments,
    /// Common task fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields
}

impl Default for CallOpenApiTaskDefinition {
    fn default() -> Self {
        Self { 
            call: "openapi".to_string(), 
            with: OpenApiArguments::default(), 
            common: TaskDefinitionFields::default() 
        }
    }
}

/// Arguments for an OpenAPI call
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenApiArguments {
    /// OpenAPI document
    #[serde(rename = "document")]
    pub document: ExternalResourceDefinition,
    /// Operation ID to call
    #[serde(rename = "operationId")]
    pub operation_id: String,
    /// Parameters mapping
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, Value>>,
    /// Authentication policy
    #[serde(rename = "authentication", skip_serializing_if = "Option::is_none")]
    pub authentication: Option<OneOfAuthenticationPolicyDefinitionOrReference>,
    /// Desired output format
    #[serde(rename = "output", skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Redirection strategy
    #[serde(rename = "redirect", skip_serializing_if = "Option::is_none")]
    pub redirect: Option<bool>
}

/// Represents the definition of a task used to call an Agent-to-Agent operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallA2aTaskDefinition {
    /// The call type (must be 'a2a')
    #[serde(rename = "call")]
    pub call: String,
    /// Arguments for the A2A call
    #[serde(rename = "with")]
    pub with: A2aArguments,
    /// Common task fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields
}

impl Default for CallA2aTaskDefinition {
    fn default() -> Self {
        Self { 
            call: "a2a".to_string(), 
            with: A2aArguments::default(), 
            common: TaskDefinitionFields::default() 
        }
    }
}

/// Arguments for an A2A call
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct A2aArguments {
    /// Agent card reference
    #[serde(rename = "agentCard", skip_serializing_if = "Option::is_none")]
    pub agent_card: Option<ExternalResourceDefinition>,
    /// Server endpoint
    #[serde(rename = "server", skip_serializing_if = "Option::is_none")]
    pub server: Option<OneOfEndpointDefinitionOrUri>,
    /// Method to call
    #[serde(rename = "method")]
    pub method: String,
    /// Parameters for the method
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>
}

/// Represents the definition of a task used to perform an MCP call
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallMcpTaskDefinition {
    /// The call type (must be 'mcp')
    #[serde(rename = "call")]
    pub call: String,
    /// Arguments for the MCP call
    #[serde(rename = "with")]
    pub with: McpArguments,
    /// Common task fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields
}

impl Default for CallMcpTaskDefinition {
    fn default() -> Self {
        Self { 
            call: "mcp".to_string(), 
            with: McpArguments::default(), 
            common: TaskDefinitionFields::default() 
        }
    }
}

/// Arguments for an MCP call
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpArguments {
    /// Protocol version
    #[serde(rename = "protocolVersion", skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    /// MCP method
    #[serde(rename = "method")]
    pub method: String,
    /// Method parameters
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// Call transport configuration
    #[serde(rename = "transport")]
    pub transport: McpTransportDefinition,
    /// Client identification
    #[serde(rename = "client", skip_serializing_if = "Option::is_none")]
    pub client: Option<McpClientDefinition>
}

/// Configuration for MCP transport
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum McpTransportDefinition {
    /// HTTP transport
    #[serde(rename = "http")]
    Http(McpHttpTransportDefinition),
    /// Stdio transport
    #[serde(rename = "stdio")]
    Stdio(McpStdioTransportDefinition)
}

impl Default for McpTransportDefinition {
    fn default() -> Self {
        McpTransportDefinition::Http(McpHttpTransportDefinition::default())
    }
}

/// HTTP transport for MCP
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpHttpTransportDefinition {
    /// HTTP endpoint
    #[serde(rename = "endpoint")]
    pub endpoint: OneOfEndpointDefinitionOrUri,
    /// Custom headers
    #[serde(rename = "headers", skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>
}

/// Stdio transport for MCP
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpStdioTransportDefinition {
    /// Command to execute
    #[serde(rename = "command")]
    pub command: String,
    /// Command arguments
    #[serde(rename = "arguments", skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<String>>,
    /// Environment variables
    #[serde(rename = "environment", skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>
}

/// Client definition for MCP
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpClientDefinition {
    /// Client name
    #[serde(rename = "name")]
    pub name: String,
    /// Client version
    #[serde(rename = "version")]
    pub version: String
}

/// Represents the definition of a generic task used to call a function
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallFunctionTaskDefinition{

    /// Gets/sets the reference to the function to call
    #[serde(rename = "call")]
    pub call: String,

    /// Gets/sets a key/value mapping of the call's arguments, if any
    #[serde(rename = "with", skip_serializing_if = "Option::is_none")]
    pub with: Option<HashMap<String, Value>>,

    /// Gets/sets a boolean indicating whether or not to wait for the called function to return. Defaults to true
    #[serde(rename = "await", skip_serializing_if = "Option::is_none")]
    pub await_: Option<bool>,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}

impl CallTaskDefinition {

    /// Gets the task's common fields
    pub fn common(&self) -> &TaskDefinitionFields {
        match self {
            Self::AsyncApi(v) => &v.common,
            Self::Grpc(v) => &v.common,
            Self::Http(v) => &v.common,
            Self::OpenApi(v) => &v.common,
            Self::A2a(v) => &v.common,
            Self::Mcp(v) => &v.common,
            Self::Function(v) => &v.common,
        }
    }

    /// Gets the task's common fields mutably
    pub fn common_mut(&mut self) -> &mut TaskDefinitionFields {
        match self {
            Self::AsyncApi(v) => &mut v.common,
            Self::Grpc(v) => &mut v.common,
            Self::Http(v) => &mut v.common,
            Self::OpenApi(v) => &mut v.common,
            Self::A2a(v) => &mut v.common,
            Self::Mcp(v) => &mut v.common,
            Self::Function(v) => &mut v.common,
        }
    }

    /// Initializes a new CalltaskDefinition
    pub fn new_function(call: &str, with: Option<HashMap<String, Value>>, await_: Option<bool>) -> Self{
        CallTaskDefinition::Function(Box::new(CallFunctionTaskDefinition { 
            call: call.to_string(), 
            with, 
            await_,
            common: TaskDefinitionFields::new()
        }))
    }

}


/// Represents the configuration of a task that is composed of multiple subtasks to run sequentially
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoTaskDefinition{

    /// Gets/sets a name/definition mapping of the subtasks to perform sequentially
    #[serde(rename = "do")]
    pub do_: Map<String, TaskDefinition>,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for DoTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::DO
    }
}
impl DoTaskDefinition {
    
    /// Initializes a new CalltaskDefinition
    pub fn new(do_: Map<String, TaskDefinition>) -> Self{
        Self { 
            do_,
            common: TaskDefinitionFields::new()
        }
    }

}

/// Represents the configuration of a task used to emit an event
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitTaskDefinition{

    /// Gets/sets the configuration of an event's emission
    #[serde(rename = "emit")]
    pub emit: EventEmissionDefinition,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields,

}
impl TaskDefinitionBase for EmitTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::EMIT
    }
}
impl EmitTaskDefinition {
    /// Initializes a new EmitTaskDefinition
    pub fn new(emit: EventEmissionDefinition) -> Self{
        Self { 
            emit,
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the configuration of an event's emission
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEmissionDefinition{

    /// Gets/sets the definition of the event to emit
    #[serde(rename = "event")]
    pub event: EventDefinition

}
impl EventEmissionDefinition {
    pub fn new(event: EventDefinition) -> Self{
        Self { 
            event 
        }
    }
}

/// <summary>
/// Represents the definition of a task that executes a set of subtasks iteratively for each element in a collection
/// </summary>
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForTaskDefinition{

    /// Gets/sets the definition of the loop that iterates over a range of values
    #[serde(rename = "for")]
    pub for_: ForLoopDefinition,

    /// Gets/sets a runtime expression that represents the condition, if any, that must be met for the iteration to continue
    #[serde(rename = "while", skip_serializing_if = "Option::is_none")]
    pub while_: Option<String>,

    /// Gets/sets the tasks to perform for each item in the collection
    #[serde(rename = "do")]
    pub do_: Map<String, TaskDefinition>,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for ForTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::FOR
    }
}
impl ForTaskDefinition {
    /// Initializes a new ForTaskDefinition
    pub fn new(for_: ForLoopDefinition, do_: Map<String, TaskDefinition>, while_: Option<String>) -> Self{
        Self { 
            for_, 
            while_, 
            do_,
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the definition of a loop that iterates over a range of values
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForLoopDefinition{

    /// Gets/sets the name of the variable that represents each element in the collection during iteration
    #[serde(rename = "each")]
    pub each: String,

    /// Gets/sets the collection to iterate over: a runtime expression (JSON string) or an inline array
    #[serde(rename = "in")]
    pub in_: Value,

    /// Gets/sets the name of the variable used to hold the index of each element in the collection during iteration
    #[serde(rename = "at", skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,

    /// Gets/sets the definition of the data, if any, to pass to iterations to run
    #[serde(rename = "input", skip_serializing_if = "Option::is_none")]
    pub input: Option<InputDataModelDefinition>,

}
impl ForLoopDefinition {
    pub fn new(each: &str, in_: &str, at: Option<String>, input: Option<InputDataModelDefinition>) -> Self{
        Self { 
            each: each.to_string(), 
            in_: Value::String(in_.to_string()),
            at,
            input
        }
    }

    /// Initializes a new ForLoopDefinition that iterates over an inline array
    pub fn new_inline(each: &str, items: Vec<Value>, at: Option<String>, input: Option<InputDataModelDefinition>) -> Self{
        Self {
            each: each.to_string(),
            in_: Value::Array(items),
            at,
            input
        }
    }
}

/// Represents the configuration of a task that is composed of multiple subtasks to run concurrently
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForkTaskDefinition{

    /// Gets/sets the configuration of the branches to perform concurrently
    #[serde(rename = "fork")]
    pub fork: BranchingDefinition,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for ForkTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::FORK
    }
}
impl ForkTaskDefinition {
    /// Initializes a new ForkTaskDefinition
    pub fn new(fork: BranchingDefinition) -> Self{
        Self { 
            fork,
            common: TaskDefinitionFields::new()
         }
    }
}

/// Represents an object used to configure branches to perform concurrently
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchingDefinition{

    /// Gets/sets a name/definition mapping of the subtasks to perform concurrently
    #[serde(rename = "branches")]
    pub branches: Map<String, TaskDefinition>,

    /// Gets/sets a boolean indicating whether or not the branches should compete each other. If `true` and if a branch completes, it will cancel all other branches then it will return its output as the task's output
    #[serde(rename = "compete")]
    pub compete: bool

}
impl BranchingDefinition{
    pub fn new(branches:Map<String, TaskDefinition>, compete: bool) -> Self{
        Self { 
            branches, 
            compete 
        }
    }
}

/// Represents the configuration of a task used to listen to specific events
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenTaskDefinition{

    /// Gets/sets the configuration of the listener to use
    #[serde(rename = "listen")]
    pub listen: ListenerDefinition,

    ///Gets/sets the configuration of the iterator, if any, for processing each consumed event
    #[serde(rename = "foreach")]
    pub foreach: Option<SubscriptionIteratorDefinition>,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for ListenTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::LISTEN
    }
}
impl ListenTaskDefinition {
    /// Initializes a new ListenTaskDefinition
    pub fn new(listen: ListenerDefinition) -> Self{
        Self { 
            listen,
            foreach: None,
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the configuration of an event listener
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenerDefinition{

    /// Gets/sets the listener's target
    #[serde(rename = "to")]
    pub to: EventConsumptionStrategyDefinition,

    /// Gets/sets a string that specifies how events are read during the listen operation
    #[serde(rename = "read")]
    pub read: Option<String>

}
impl ListenerDefinition {
    pub fn new(to: EventConsumptionStrategyDefinition) -> Self{
        Self{
            to,
            read: None
        }
    }
}

/// Represents the configuration of a task used to listen to specific events
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaiseTaskDefinition{

    /// Gets/sets the definition of the error to raise
    #[serde(rename = "raise")]
    pub raise: RaiseErrorDefinition,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for RaiseTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::RAISE
    }
}
impl RaiseTaskDefinition {
    /// Initializes a new RaiseTaskDefinition
    pub fn new(raise: RaiseErrorDefinition) -> Self{
        Self{
            raise,
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the definition of the error to raise
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaiseErrorDefinition{

    /// Gets/sets the error to raise
    #[serde(rename = "error")]
    pub error: OneOfErrorDefinitionOrReference

}
impl RaiseErrorDefinition{

    /// Initializes a new RaiseErrorDefinition
    pub fn new(error: OneOfErrorDefinitionOrReference) -> Self{
        Self { error }
    }

}

/// Represents the configuration of a task used to run a given process
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTaskDefinition{

    /// Gets/sets the configuration of the process to execute
    #[serde(rename = "run")]
    pub run: ProcessTypeDefinition,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for RunTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::RUN
    }
}
impl RunTaskDefinition {
    /// Initializes a new RunTaskDefinition
    pub fn new(run: ProcessTypeDefinition) -> Self{
        Self { 
            run,
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the configuration of a process execution
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessTypeDefinition{

    /// Gets/sets the configuration of the container to run
    #[serde(rename = "container", skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerProcessDefinition>,

    /// Gets/sets the configuration of the script to run
    #[serde(rename = "script", skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptProcessDefinition>,

    /// Gets/sets the configuration of the shell command to run
    #[serde(rename = "shell", skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellProcessDefinition>,

    /// Gets/sets the configuration of the workflow to run
    #[serde(rename = "workflow", skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowProcessDefinition>,

    /// Gets/sets a boolean indicating whether or not to await the process completion before continuing. Defaults to 'true'
    #[serde(rename = "await", skip_serializing_if = "Option::is_none")]
    pub await_: Option<bool>,

    /// Gets/sets the configuration of the process output. Defaults to 'stdout'
    #[serde(rename = "return", skip_serializing_if = "Option::is_none")]
    pub return_: Option<String>

}
impl ProcessTypeDefinition {

    /// Creates a new container process
    pub fn using_container(container: ContainerProcessDefinition, await_: Option<bool>, return_: Option<String>) -> Self{
        Self { 
            container: Some(container),
            await_,
            return_,
            shell: None,
            script: None,
            workflow: None
        }
    }

    /// Creates a new script process
    pub fn using_script(script: ScriptProcessDefinition, await_: Option<bool>, return_: Option<String>) -> Self{
        Self { 
            script: Some(script),
            await_,
            return_,
            container: None,
            shell: None,
            workflow: None
        }
    }

    /// Creates a new shell process
    pub fn using_shell(shell: ShellProcessDefinition, await_: Option<bool>, return_: Option<String>) -> Self{
        Self { 
            shell: Some(shell),
            await_,
            return_,
            container: None,
            script: None,
            workflow: None
        }
    }

    /// Creates a new workflow process
    pub fn using_workflow(workflow: WorkflowProcessDefinition, await_: Option<bool>, return_: Option<String>) -> Self{
        Self { 
            workflow: Some(workflow),
            await_,
            return_,
            container: None,
            shell: None,
            script: None
        }
    }
    
    /// Gets the type of the defined process
    pub fn get_process_type(&self) -> &str{
        if self.container.is_some(){
            ProcessType::CONTAINER
        }
        else if self.script.is_some(){
            ProcessType::SCRIPT
        }
        else if self.shell.is_some(){
            ProcessType::SHELL
        }
        else{
            ProcessType::WORKFLOW
        }
    }

}

/// Represents the configuration of a container process
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerProcessDefinition{

    /// Gets/sets the name of the container image to run
    #[serde(rename = "image")]
    pub image: String,

    /// Gets/sets the name of the container to run
    #[serde(rename = "name")]
    pub name: Option<String>,

    /// Gets/sets the command, if any, to execute on the container
    #[serde(rename = "command", skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Gets/sets a list containing the container's port mappings, if any
    #[serde(rename = "ports", skip_serializing_if = "Option::is_none")]
    pub ports: Option<HashMap<u16, u16>>,

    /// Gets/sets the volume mapping for the container, if any
    #[serde(rename = "volumes", skip_serializing_if = "Option::is_none")]
    pub volumes: Option<HashMap<String, String>>,

    /// Gets/sets a key/value mapping of the environment variables, if any, to use when running the configured process
    #[serde(rename = "environment", skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,

    /// Gets/sets the data to pass to the process via stdin, if any
    #[serde(rename = "stdin", skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,

    /// Gets/sets a list of arguments, if any, to pass to the container (argv)
    #[serde(rename = "arguments", skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<String>>,
}
impl ContainerProcessDefinition {
    pub fn new(image: &str, name: Option<String>, command: Option<String>, ports: Option<HashMap<u16, u16>>, volumes: Option<HashMap<String, String>>, environment: Option<HashMap<String, String>>, stdin: Option<String>, arguments: Option<Vec<String>>) -> Self{
        Self { 
            image: image.to_string(), 
            name,
            command, 
            ports, 
            volumes, 
            environment,
            stdin,
            arguments,
        }
    }
}

/// Represents the definition of a script evaluation process
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptProcessDefinition{

    /// Gets/sets the language of the script to run
    #[serde(rename = "language")]
    pub language: String,

    /// Gets/sets the script's code. Required if 'source' has not been set
    #[serde(rename = "code", skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,

    /// Gets the the script's source. Required if 'code' has not been set
    #[serde(rename = "source", skip_serializing_if = "Option::is_none")]
    pub source: Option<ExternalResourceDefinition>,

    /// Gets/sets the data to pass to the process via stdin
    #[serde(rename = "stdin", skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,

    /// Gets/sets the arguments, if any, to pass to the script
    #[serde(rename = "arguments", skip_serializing_if = "Option::is_none")]
    pub arguments: Option<ScriptArguments>,

    /// Gets/sets a key/value mapping of the environment variables, if any, to use when running the configured process
    #[serde(rename = "environment", skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,

}

/// Represents the arguments of a script: a key/value mapping (DSL 1.0.3), or a positional list (earlier DSL versions)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScriptArguments {
    /// A key/value mapping of named arguments
    Map(HashMap<String, Value>),
    /// A list of positional arguments (argv)
    List(Vec<String>),
}

impl ScriptProcessDefinition {

    /// Initializes a new script from code
    pub fn from_code(language: &str, code: String, stdin: Option<String>, arguments: Option<ScriptArguments>, environment: Option<HashMap<String, String>>) -> Self{
        Self {
            language: language.to_string(),
            code: Some(code),
            source: None,
            stdin,
            arguments,
            environment
         }
    }

    /// Initializes a new script from an external resource
    pub fn from_source(language: &str, source: ExternalResourceDefinition, stdin: Option<String>, arguments: Option<ScriptArguments>, environment: Option<HashMap<String, String>>) -> Self{
        Self {
            language: language.to_string(),
            code: None,
            source: Some(source),
            stdin,
            arguments,
            environment
         }
    }
}

/// Represents the definition of a shell process
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellProcessDefinition{
    
    /// Gets/sets the shell command to run
    #[serde(rename = "command")]
    pub command: String,

    /// Gets/sets a runtime expression, if any, to pass to the shell command as standard input (stdin)
    #[serde(rename = "stdin", skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,

    /// Gets/sets the arguments of the shell command to run
    #[serde(rename = "arguments", skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<String>>,

    /// Gets/sets the working directory, if any, in which to run the shell command
    #[serde(rename = "directory", skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,

    /// Gets/sets a key/value mapping of the environment variables, if any, to use when running the configured process
    #[serde(rename = "environment", skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,

}
impl ShellProcessDefinition {
    pub fn new(command: &str, arguments: Option<Vec<String>>, environment: Option<HashMap<String, String>>) -> Self{
        Self {
            command: command.to_string(),
            stdin: None,
            arguments,
            directory: None,
            environment
        }
    }
}

/// Represents the definition of a (sub)workflow process
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowProcessDefinition{
    
    /// Gets/sets the namespace the workflow to run belongs to
    #[serde(rename = "namespace")]
    pub namespace: String,

    /// Gets/sets the name of the workflow to run
    #[serde(rename = "name")]
    pub name: String,

    /// Gets/sets the version of the workflow to run
    #[serde(rename = "version")]
    pub version: String,

    /// Gets/sets the data, if any, to pass as input to the workflow to execute. The value should be validated against the target workflow's input schema, if specified
    #[serde(rename = "input", skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>

}
impl WorkflowProcessDefinition {
    pub fn new(namespace: &str, name: &str, version: &str, input: Option<Value>) -> Self{
        Self { 
            namespace: namespace.to_string(), 
            name: name.to_string(), 
            version: version.to_string(), 
            input
        }
    }
}

/// Represents the value that can be set in a Set task - either a map or a runtime expression string
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SetValue {
    /// A map of key-value pairs to set
    Map(HashMap<String, Value>),
    /// A runtime expression string that evaluates to the data to set
    Expression(String),
}

impl Default for SetValue {
    fn default() -> Self {
        SetValue::Map(HashMap::new())
    }
}

/// Represents the definition of a task used to set data
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetTaskDefinition{

    /// Gets/sets the data to set
    #[serde(rename = "set")]
    pub set: SetValue,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for SetTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::SET
    }
}
impl SetTaskDefinition {
    /// Initializes a new SetTaskDefinition
    pub fn new() -> Self{
        Self {
            set: SetValue::Map(HashMap::new()),
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the definition of a task that evaluates conditions and executes specific branches based on the result
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchTaskDefinition{

    /// Gets/sets the definition of the switch to use
    #[serde(rename = "switch")]
    pub switch: Map<String, SwitchCaseDefinition>,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for SwitchTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::SWITCH
    }
}
impl SwitchTaskDefinition {
    /// Initializes a new SwitchTaskDefinition
    pub fn new() -> Self{
        Self { 
            switch: Map::new(),
            common: TaskDefinitionFields::new()
        }
    }
}

/// Represents the definition of a case within a switch task, defining a condition and corresponding tasks to execute if the condition is met
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchCaseDefinition{

    /// Gets/sets the condition that determines whether or not the case should be executed in a switch task
    #[serde(rename = "when", skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,

    /// Gets/sets the transition to perform when the case matches
    #[serde(rename = "then", skip_serializing_if = "Option::is_none")]
    pub then: Option<String>

}

/// Represents the definition of a task used to try one or more subtasks, and to catch/handle the errors that can potentially be raised during execution
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TryTaskDefinition{

    /// Gets/sets a name/definition map of the tasks to try running
    #[serde(rename = "try")]
    pub try_: Map<String, TaskDefinition>,

    /// Gets/sets the object used to define the errors to catch
    #[serde(rename = "catch")]
    pub catch: ErrorCatcherDefinition,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for TryTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::TRY
    }
}
impl TryTaskDefinition {
    
    /// Initializes a new TryTaskDefintion
    pub fn new(try_: Map<String, TaskDefinition>, catch: ErrorCatcherDefinition) -> Self{
        Self { 
            try_,
            catch,
            common: TaskDefinitionFields::new()
        }
    }

}

/// Represents the configuration of a concept used to catch errors
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorCatcherDefinition{

    /// Gets/sets the definition of the errors to catch
    #[serde(rename = "errors", skip_serializing_if = "Option::is_none")]
    pub errors: Option<ErrorFilterDefinition>,

    /// Gets/sets the name of the runtime expression variable to save the error as. Defaults to 'error'.
    #[serde(rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_: Option<String>,

    /// Gets/sets a runtime expression used to determine whether or not to catch the filtered error
    #[serde(rename = "when", skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,

    /// Gets/sets a runtime expression used to determine whether or not to catch the filtered error
    #[serde(rename = "exceptWhen", skip_serializing_if = "Option::is_none")]
    pub except_when: Option<String>,

    /// Gets/sets the retry policy to use, if any
    #[serde(rename = "retry", skip_serializing_if = "Option::is_none")]
    pub retry: Option<OneOfRetryPolicyDefinitionOrReference>,

    /// Gets/sets a name/definition map of the tasks, if any, to run when catching an error
    #[serde(rename = "do", skip_serializing_if = "Option::is_none")]
    pub do_: Option<Map<String, TaskDefinition>>

}

/// Represents the definition an an error filter
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorFilterDefinition{

    /// Gets/sets a key/value mapping of the properties errors to filter must define
    #[serde(rename = "with", skip_serializing_if = "Option::is_none")]
    pub with: Option<HashMap<String, Value>>

}

/// Represents the definition of a task used to wait a certain amount of time
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitTaskDefinition{

    /// Gets/sets the amount of time to wait before resuming workflow
    #[serde(rename = "wait")]
    pub wait: OneOfDurationOrIso8601Expression,

    /// Gets/sets the task's common fields
    #[serde(flatten)]
    pub common: TaskDefinitionFields

}
impl TaskDefinitionBase for WaitTaskDefinition {
    fn task_type(&self) -> &str {
        TaskType::WAIT
    }
}
impl WaitTaskDefinition {

    /// Initializes a new WaitTaskDefinition
    pub fn new(wait: OneOfDurationOrIso8601Expression) -> Self{
        Self {
            wait,
            common: TaskDefinitionFields::new()
        }
    }

}

/// Represents the definition of the iterator used to process each event or message consumed by a subscription
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionIteratorDefinition{

    /// Gets the name of the variable used to store the current item being enumerated
    #[serde(rename = "item")]
    pub item: Option<String>,

    /// Gets the name of the variable used to store the index of the current item being enumerated
    #[serde(rename = "at")]
    pub at: Option<String>,

    /// Gets the tasks to perform for each consumed item
    #[serde(rename = "do")]
    pub do_: Option<Map<String, TaskDefinition>>,

    /// Gets/sets an object, if any, used to customize the item's output and to document its schema.
    #[serde(rename = "output", skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputDataModelDefinition>,

    /// Gets/sets an object, if any, used to customize the content of the workflow context.
    #[serde(rename = "export", skip_serializing_if = "Option::is_none")]
    pub export: Option<OutputDataModelDefinition>

}
impl SubscriptionIteratorDefinition{

    /// Initializes a new SubscriptionIteratorDefinition
    pub fn new() -> Self{
        SubscriptionIteratorDefinition::default()
    }

}