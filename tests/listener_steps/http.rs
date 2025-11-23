use crate::ListenerWorld;
use crate::common::parse_docstring;
use cucumber::{given, when, then};
use serde_json::Value;
use serverless_workflow_core::models::workflow::WorkflowDefinition;

// Helper to parse JSON/YAML request body
fn parse_request_body(text: &str) -> serde_json::Value {
    // Try JSON first, then YAML
    serde_json::from_str(text)
        .or_else(|_| serde_yaml::from_str(text).map_err(|e| e.to_string()))
        .unwrap_or_else(|_| serde_json::json!({}))
}

// HTTP POST Python Add request
#[given(regex = r#"^(?:given )?the HTTP POST python add request body for "([^"]+)" is:$"#)]
async fn given_http_post_python_add_request(world: &mut ListenerWorld, path: String, step: &cucumber::gherkin::Step) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_request_body(&request_text);
    world.http_requests.insert(path, request);
}

// HTTP POST Python Multiply request
#[given(regex = r#"^(?:given )?the HTTP POST python multiply request body for "([^"]+)" is:$"#)]
async fn given_http_post_python_multiply_request(world: &mut ListenerWorld, path: String, step: &cucumber::gherkin::Step) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_request_body(&request_text);
    world.http_requests.insert(path, request);
}

// HTTP POST TypeScript Add request
#[given(regex = r#"^(?:given )?the HTTP POST typescript add request body for "([^"]+)" is:$"#)]
async fn given_http_post_typescript_add_request(world: &mut ListenerWorld, path: String, step: &cucumber::gherkin::Step) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_request_body(&request_text);
    world.http_requests.insert(path, request);
}

// HTTP POST TypeScript Multiply request
#[given(regex = r#"^(?:given )?the HTTP POST typescript multiply request body for "([^"]+)" is:$"#)]
async fn given_http_post_typescript_multiply_request(world: &mut ListenerWorld, path: String, step: &cucumber::gherkin::Step) {
    let request_text = parse_docstring(step.docstring.as_ref().unwrap());
    let request = parse_request_body(&request_text);
    world.http_requests.insert(path, request);
}

// HTTP Python Add endpoint invocation
#[when(regex = r#"^the HTTP python add endpoint "([^"]+)" is called$"#)]
async fn when_http_python_add_endpoint_called(world: &mut ListenerWorld, path: String) {
    // First, start the workflow if it hasn't been started yet
    if world.instance_id.is_none() {
        let workflow_yaml = world.workflow_definition.as_ref().expect("No workflow definition");
        let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml)
            .expect("Failed to parse workflow");

        let engine = world.engine.as_ref().expect("No engine");
        let instance_id = engine.start(workflow).await.expect("Failed to start workflow");

        // Wait a bit for listeners to start
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        world.instance_id = Some(instance_id);
    }

    // Extract just the path from "POST /api/v1/add"
    let path_only = path.split_whitespace().last().unwrap_or(&path);
    let request = world.http_requests.get(path_only).cloned().unwrap_or(serde_json::json!({}));

    // Send HTTP request to the listener
    let client = reqwest::Client::new();
    let response = client.post(&format!("http://localhost:8080{}", path_only))
        .json(&request)
        .send()
        .await
        .expect("Failed to send HTTP request");

    let status = response.status().as_u16();
    let body: serde_json::Value = response.json().await.expect("Failed to parse response");

    world.http_response_status = Some(status);
    world.http_responses.insert(path_only.to_string(), body);
}

// HTTP Python Multiply endpoint invocation
#[when(regex = r#"^the HTTP python multiply endpoint "([^"]+)" is called$"#)]
async fn when_http_python_multiply_endpoint_called(world: &mut ListenerWorld, path: String) {
    // Reuse the same logic as add endpoint
    when_http_python_add_endpoint_called(world, path).await;
}

// HTTP TypeScript Add endpoint invocation
#[when(regex = r#"^the HTTP typescript add endpoint "([^"]+)" is called$"#)]
async fn when_http_typescript_add_endpoint_called(world: &mut ListenerWorld, path: String) {
    // Reuse the same logic as Python add endpoint
    when_http_python_add_endpoint_called(world, path).await;
}

// HTTP TypeScript Multiply endpoint invocation
#[when(regex = r#"^the HTTP typescript multiply endpoint "([^"]+)" is called$"#)]
async fn when_http_typescript_multiply_endpoint_called(world: &mut ListenerWorld, path: String) {
    // Reuse the same logic as Python add endpoint
    when_http_python_add_endpoint_called(world, path).await;
}

// HTTP response status verification
#[then(regex = r#"^the HTTP response status should be (\d+)$"#)]
async fn then_http_response_status(world: &mut ListenerWorld, expected_status: u16) {
    let actual_status = world.http_response_status.expect("No HTTP response status recorded");

    // If status is 500, print the error response for debugging
    if actual_status == 500 {
        if let Some(response_body) = world.http_responses.values().last() {
            eprintln!("HTTP 500 error response: {}", serde_json::to_string_pretty(response_body).unwrap());
        }
    }

    assert_eq!(actual_status, expected_status, "HTTP response status mismatch");
}

// HTTP response body verification
#[then(expr = "the HTTP response body should be:")]
async fn then_http_response_body(world: &mut ListenerWorld, step: &cucumber::gherkin::Step) {
    let expected_text = parse_docstring(step.docstring.as_ref().unwrap());
    let expected: serde_json::Value = parse_request_body(&expected_text);

    // Get the last response (we should have only one in the simple case)
    let actual = world.http_responses.values().last().expect("No HTTP response recorded");

    assert_eq!(actual, &expected, "HTTP response body mismatch");
}
