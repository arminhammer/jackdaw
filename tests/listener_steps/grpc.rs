#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::wildcard_enum_match_arm)]
#![allow(clippy::single_match)]
#![allow(clippy::for_kv_map)]

use crate::ListenerWorld;
use crate::common::parse_docstring;
use cucumber::{given, then, when};
use jackdaw::workflow_source::StringSource;
use prost::Message;
use snafu::prelude::*;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("gRPC test error: {message}"))]
    GrpcTest { message: String },

    #[snafu(display("Workflow YAML error: {source}"))]
    WorkflowYaml { source: serde_yaml::Error },

    #[snafu(display("Engine error: {source}"))]
    Engine {
        source: jackdaw::durableengine::Error,
    },

    #[snafu(display("Protobuf compilation error: {source}"))]
    Protobuf { source: protox::Error },

    #[snafu(display("Protobuf descriptor error: {source}"))]
    ProtobufDescriptor {
        source: prost_reflect::DescriptorError,
    },

    #[snafu(display("HTTP request error: {source}"))]
    HttpRequest { source: reqwest::Error },

    #[snafu(display("Prost decode error: {source}"))]
    ProstDecode { source: prost::DecodeError },

    #[snafu(display("Prost encode error: {source}"))]
    ProstEncode { source: prost::EncodeError },
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<serde_yaml::Error> for Error {
    fn from(source: serde_yaml::Error) -> Self {
        Error::WorkflowYaml { source }
    }
}

impl From<jackdaw::durableengine::Error> for Error {
    fn from(source: jackdaw::durableengine::Error) -> Self {
        Error::Engine { source }
    }
}

impl From<protox::Error> for Error {
    fn from(source: protox::Error) -> Self {
        Error::Protobuf { source }
    }
}

impl From<prost_reflect::DescriptorError> for Error {
    fn from(source: prost_reflect::DescriptorError) -> Self {
        Error::ProtobufDescriptor { source }
    }
}

impl From<reqwest::Error> for Error {
    fn from(source: reqwest::Error) -> Self {
        Error::HttpRequest { source }
    }
}

impl From<prost::DecodeError> for Error {
    fn from(source: prost::DecodeError) -> Self {
        Error::ProstDecode { source }
    }
}

impl From<prost::EncodeError> for Error {
    fn from(source: prost::EncodeError) -> Self {
        Error::ProstEncode { source }
    }
}

// Helper to parse proto text format to JSON
fn parse_proto_text(text: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line == "proto" {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            // Try to parse as number, otherwise treat as string
            if let Ok(num) = value.parse::<i64>() {
                map.insert(key.to_string(), serde_json::json!(num));
            } else if let Ok(num) = value.parse::<f64>() {
                map.insert(key.to_string(), serde_json::json!(num));
            } else {
                map.insert(key.to_string(), serde_json::json!(value));
            }
        }
    }
    serde_json::Value::Object(map)
}

// gRPC Python Add request
#[given(regex = r#"^(?:given )?the gRPC add python request for "([^"]+)" is:$"#)]
async fn given_grpc_add_python_request(
    world: &mut ListenerWorld,
    method: String,
    step: &cucumber::gherkin::Step,
) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_proto_text(&request_text);
    world.grpc_requests.insert(method, request);
}

// gRPC Python Multiply request
#[given(regex = r#"^(?:given )?the gRPC multiply python request for "([^"]+)" is:$"#)]
async fn given_grpc_multiply_python_request(
    world: &mut ListenerWorld,
    method: String,
    step: &cucumber::gherkin::Step,
) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_proto_text(&request_text);
    world.grpc_requests.insert(method, request);
}

// gRPC TypeScript Add request
#[given(regex = r#"^(?:given )?the gRPC typescript add request for "([^"]+)" is:$"#)]
async fn given_grpc_add_typescript_request(
    world: &mut ListenerWorld,
    method: String,
    step: &cucumber::gherkin::Step,
) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_proto_text(&request_text);
    world.grpc_requests.insert(method, request);
}

// gRPC TypeScript Multiply request
#[given(regex = r#"^(?:given )?the gRPC typescript multiply request for "([^"]+)" is:$"#)]
async fn given_grpc_multiply_typescript_request(
    world: &mut ListenerWorld,
    method: String,
    step: &cucumber::gherkin::Step,
) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_proto_text(&request_text);
    world.grpc_requests.insert(method, request);
}

// Helper to execute workflow and make actual gRPC call
async fn execute_workflow_and_call_grpc(world: &mut ListenerWorld, method: String) -> Result<()> {
    // Parse workflow
    let workflow_yaml = world
        .workflow_definition
        .as_ref()
        .ok_or_else(|| Error::GrpcTest {
            message: "No workflow definition".to_string(),
        })?;

    // Get engine
    let engine = world.engine.as_ref().ok_or_else(|| Error::GrpcTest {
        message: "No engine".to_string(),
    })?;

    // Start workflow execution with a race against our gRPC call
    // The workflow will start listeners and then block forever on `until: false`
    // We use spawn_local since the workflow future is !Send
    let engine_clone = engine.clone();
    let workflow_yaml_clone = workflow_yaml.clone();

    // Create an abortable workflow task
    let (abort_handle, abort_registration) = futures::future::AbortHandle::new_pair();
    let workflow_future = futures::future::Abortable::new(
        async move {
            let source = StringSource::new(workflow_yaml_clone);
            if let Ok(mut handle) = engine_clone.execute(source, serde_json::json!({})).await {
                // For perpetual workflows, consume events to keep it running
                while let Some(_event) = handle.next_event().await {
                    // Process events as they come
                }
            }
        },
        abort_registration,
    );

    // Start workflow in background using spawn_local (works with LocalSet)
    tokio::task::spawn_local(workflow_future);

    // Wait for listeners to start
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

    // Get the request payload
    let request_json = world
        .grpc_requests
        .get(&method)
        .ok_or_else(|| Error::GrpcTest {
            message: format!("No request for method {}", method),
        })?
        .clone();

    // Make actual gRPC call using tonic and prost-reflect
    // Parse method string to extract service and endpoint (e.g., "calculator.Calculator/Add")
    let endpoint = "http://localhost:50051"; // From workflow definition

    // Load and compile proto file at runtime using protox
    let proto_path = "tests/fixtures/listeners/specs/calculator.proto";
    let file_descriptor_set = protox::compile([proto_path], ["."])?;
    let pool = prost_reflect::DescriptorPool::from_file_descriptor_set(file_descriptor_set)?;

    // Find the service and method
    let parts: Vec<&str> = method.split('/').collect();
    if parts.len() != 2 {
        return Err(Error::GrpcTest {
            message: format!("Invalid method format: {}", method),
        });
    }
    let service_name = parts[0];
    let method_name = parts[1];

    let service = pool
        .get_service_by_name(service_name)
        .ok_or_else(|| Error::GrpcTest {
            message: format!("Service not found: {}", service_name),
        })?;
    let method_desc = service
        .methods()
        .find(|m| m.name() == method_name)
        .ok_or_else(|| Error::GrpcTest {
            message: format!("Method not found: {}", method_name),
        })?;

    // Create DynamicMessage from JSON properly using prost-reflect
    let input_descriptor = method_desc.input();
    let mut request_msg = prost_reflect::DynamicMessage::new(input_descriptor.clone());

    // Set fields from JSON
    for (key, value) in request_json.as_object().unwrap() {
        if let Some(field) = input_descriptor.get_field_by_name(key) {
            let field_value = match value {
                serde_json::Value::Number(n) if field.kind().as_message().is_none() => {
                    if let Some(i) = n.as_i64() {
                        prost_reflect::Value::I32(i as i32)
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            request_msg.set_field(&field, field_value);
        }
    }

    // Encode request to bytes
    let mut request_bytes = Vec::new();
    request_msg.encode(&mut request_bytes)?;

    // Make HTTP request directly to avoid double-encoding issues with tonic codecs
    // Create gRPC frame: [compression flag (1 byte)][length (4 bytes BE)][message]
    let msg_len = request_bytes.len() as u32;
    let mut framed_request = Vec::with_capacity(5 + request_bytes.len());
    framed_request.push(0); // No compression
    framed_request.extend_from_slice(&msg_len.to_be_bytes());
    framed_request.extend_from_slice(&request_bytes);

    // Use reqwest with HTTP/2 for gRPC
    let client = reqwest::Client::builder().http2_prior_knowledge().build()?;
    let http_response = client
        .post(format!("{}/{}/{}", endpoint, service_name, method_name))
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(framed_request)
        .send()
        .await?;

    // Read response body
    let response_body = http_response.bytes().await?;

    // Skip gRPC frame header (5 bytes)
    let response_bytes: Vec<u8> = if response_body.len() > 5 {
        response_body[5..].to_vec()
    } else {
        response_body.to_vec()
    };
    let response_msg =
        prost_reflect::DynamicMessage::decode(method_desc.output(), response_bytes.as_slice())?;

    // Convert DynamicMessage to JSON using reflect
    use prost_reflect::ReflectMessage;
    let mut response_json_map = serde_json::Map::new();
    let descriptor = response_msg.descriptor();
    for field in descriptor.fields() {
        let value = response_msg.get_field(&field);
        match value.as_ref() {
            prost_reflect::Value::I32(i) => {
                response_json_map.insert(field.name().to_string(), serde_json::json!(i));
            }
            _ => {}
        }
    }
    let response_json = serde_json::Value::Object(response_json_map);

    world.grpc_responses.insert(method, response_json);

    // Abort the workflow task now that we have our response
    abort_handle.abort();

    Ok(())
}

// gRPC Python Add method invocation
#[when(regex = r#"^the gRPC add python method "([^"]+)" is called$"#)]
async fn when_grpc_add_python_method_called(world: &mut ListenerWorld, method: String) {
    execute_workflow_and_call_grpc(world, method)
        .await
        .expect("Failed to execute workflow and call gRPC method");
}

// gRPC Python Multiply method invocation
#[when(regex = r#"^the gRPC multiply python method "([^"]+)" is called$"#)]
async fn when_grpc_multiply_python_method_called(world: &mut ListenerWorld, method: String) {
    execute_workflow_and_call_grpc(world, method)
        .await
        .expect("Failed to execute workflow and call gRPC method");
}

// gRPC TypeScript Add method invocation
#[when(regex = r#"^the gRPC typescript add method "([^"]+)" is called$"#)]
async fn when_grpc_add_typescript_method_called(world: &mut ListenerWorld, method: String) {
    execute_workflow_and_call_grpc(world, method)
        .await
        .expect("Failed to execute workflow and call gRPC method");
}

// gRPC TypeScript Multiply method invocation
#[when(regex = r#"^the gRPC typescript multiply method "([^"]+)" is called$"#)]
async fn when_grpc_multiply_typescript_method_called(world: &mut ListenerWorld, method: String) {
    execute_workflow_and_call_grpc(world, method)
        .await
        .expect("Failed to execute workflow and call gRPC method");
}

// gRPC response verification
#[then(expr = "the gRPC response should be:")]
async fn then_grpc_response(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let expected_text = parse_docstring(step.docstring.as_ref().unwrap());
    let expected = parse_proto_text(&expected_text);

    // Find matching response
    let mut found_match = false;
    for (_method, actual) in &world.grpc_responses {
        if actual == &expected {
            found_match = true;
            break;
        }
    }

    assert!(
        found_match,
        "Expected gRPC response not found.\nExpected: {:?}\nActual responses: {:?}",
        expected, world.grpc_responses
    );
}
