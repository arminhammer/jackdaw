use crate::context::Context;
use crate::executor::{Error, Executor, Result};
use async_trait::async_trait;
use openapiv3::{OpenAPI, ParameterKind, ReferenceOr, VersionedOpenAPI, v2};
use reqwest::Url;

pub struct OpenApiExecutor(pub reqwest::Client);

#[async_trait]
impl Executor for OpenApiExecutor {
    async fn exec(
        &self,
        task_name: &str,
        params: &serde_json::Value,
        ctx: &Context,
    ) -> Result<serde_json::Value> {
        // Extract document endpoint
        let doc_endpoint = params
            .get("document")
            .and_then(|d| d.get("endpoint"))
            .and_then(|e| e.as_str())
            .ok_or(Error::Execution {
                message: "No document endpoint specified".to_string(),
            })?;

        // Extract operation ID
        let operation_id =
            params
                .get("operationId")
                .and_then(|o| o.as_str())
                .ok_or(Error::Execution {
                    message: "No operationId specified".to_string(),
                })?;

        // Extract and evaluate parameters
        let parameters_raw = params
            .get("parameters")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let parameters = evaluate_parameters(&parameters_raw, ctx).await?;

        // Check for output mode (default is "content")
        let output_mode = params
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("content");

        println!("  OpenAPI call: {} at {}", operation_id, doc_endpoint);

        // Fetch the OpenAPI spec
        let spec_text = self
            .0
            .get(doc_endpoint)
            .send()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Failed to fetch OpenAPI spec: {}", e),
            })?
            .text()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Failed to read spec text: {}", e),
            })?;

        println!(
            "  Fetched spec (first 200 chars): {}",
            &spec_text.chars().take(200).collect::<String>()
        );

        // Parse as JSON value first to check version
        let spec_value: serde_json::Value = serde_json::from_str(&spec_text)
            .or_else(|_| serde_yaml::from_str(&spec_text))
            .map_err(|e| Error::Execution {
                message: format!("Failed to parse spec as JSON or YAML: {}", e),
            })?;

        // Check if it's a Swagger 2.0 spec and convert it manually
        if spec_value
            .get("swagger")
            .and_then(|v| v.as_str())
            .map(|s| s.starts_with("2."))
            == Some(true)
        {
            println!("  Detected Swagger 2.0 spec, converting to OpenAPI 3.x");
            return execute_swagger_v2_spec(
                &self.0,
                task_name,
                operation_id,
                &parameters,
                &spec_value,
                output_mode,
                doc_endpoint,
            )
            .await;
        }

        // Try to parse as VersionedOpenAPI (fallback)
        let versioned_spec: VersionedOpenAPI =
            serde_json::from_value(spec_value.clone()).map_err(|e| Error::Execution {
                message: format!("Failed to deserialize as OpenAPI spec: {}", e),
            })?;

        // Upgrade to OpenAPI 3.x if it's a Swagger 2.0 spec
        let spec: OpenAPI = versioned_spec.upgrade();

        println!("  Parsed OpenAPI spec successfully");

        execute_openapi_v3_spec(
            &self.0,
            task_name,
            operation_id,
            &parameters,
            &spec,
            output_mode,
            doc_endpoint,
        )
        .await
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

async fn execute_swagger_v2_spec(
    client: &reqwest::Client,
    task_name: &str,
    operation_id: &str,
    parameters: &serde_json::Value,
    spec_value: &serde_json::Value,
    output_mode: &str,
    doc_endpoint: &str,
) -> Result<serde_json::Value> {
    // Find operation by operationId in the spec
    let paths = spec_value
        .get("paths")
        .and_then(|p| p.as_object())
        .ok_or(Error::Execution {
            message: "No paths in Swagger spec".to_string(),
        })?;

    let mut found_operation: Option<(&str, &str, &serde_json::Value)> = None;

    for (path, path_item) in paths {
        if let Some(path_obj) = path_item.as_object() {
            for (method, operation) in path_obj {
                if let Some(op_id) = operation.get("operationId").and_then(|v| v.as_str()) {
                    if op_id == operation_id {
                        found_operation = Some((path.as_str(), method.as_str(), operation));
                        break;
                    }
                }
            }
            if found_operation.is_some() {
                break;
            }
        }
    }

    let (path_pattern, method, operation) = found_operation.ok_or(Error::Execution {
        message: format!("Operation '{}' not found in Swagger spec", operation_id),
    })?;

    // Build base URL
    let schemes = spec_value
        .get("schemes")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .unwrap_or("https");

    let host = spec_value
        .get("host")
        .and_then(|h| h.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            // Extract from doc endpoint
            Url::parse(doc_endpoint)
                .ok()
                .and_then(|u| u.host_str().map(|s| s.to_string()))
        })
        .unwrap_or_default();

    let base_path = spec_value
        .get("basePath")
        .and_then(|b| b.as_str())
        .unwrap_or("");

    let base_url = format!("{}://{}{}", schemes, host, base_path);

    // Build URL with path and query parameters
    let mut url = format!("{}{}", base_url, path_pattern);
    let mut query_params = Vec::new();

    println!("  Parameters: {:?}", parameters);

    // Process parameters
    if let Some(params_array) = operation.get("parameters").and_then(|p| p.as_array()) {
        for param in params_array {
            if let Some(param_name) = param.get("name").and_then(|n| n.as_str()) {
                if let Some(value) = parameters.get(param_name) {
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => value.to_string(),
                    };

                    let param_in = param.get("in").and_then(|i| i.as_str()).unwrap_or("");
                    match param_in {
                        "path" => {
                            url = url.replace(&format!("{{{}}}", param_name), &value_str);
                        }
                        "query" => {
                            query_params.push(format!("{}={}", param_name, value_str));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if !query_params.is_empty() {
        url = format!("{}?{}", url, query_params.join("&"));
    }

    println!("  Request: {} {}", method.to_uppercase(), url);

    // Make the HTTP request
    let response = match method.to_uppercase().as_str() {
        "GET" => client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Request failed: {}", e),
            })?,
        "POST" => {
            let body = parameters
                .get("body")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Execution {
                    message: format!("Request failed: {}", e),
                })?
        }
        "PUT" => {
            let body = parameters
                .get("body")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            client
                .put(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Execution {
                    message: format!("Request failed: {}", e),
                })?
        }
        "DELETE" => client
            .delete(&url)
            .send()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Request failed: {}", e),
            })?,
        _ => {
            return Err(Error::Execution {
                message: format!("Unsupported HTTP method: {}", method),
            });
        }
    };

    let status = response.status();
    let headers = response.headers().clone();

    println!("  Response status: {}", status);

    if !status.is_success() {
        let error_obj = serde_json::json!({
            "type": "https://serverlessworkflow.io/dsl/errors/types/communication",
            "status": status.as_u16(),
            "title": format!("HTTP {} Error", status.as_u16()),
            "detail": format!("{} request to {} failed with status {}", method.to_uppercase(), url, status),
            "instance": format!("/do/0/{}", task_name)
        });
        return Err(Error::Execution {
            message: serde_json::to_string(&error_obj).map_err(|e| Error::Execution {
                message: format!("Failed to serialize error: {}", e),
            })?,
        });
    }

    // Get response body
    let body_text = response.text().await.map_err(|e| Error::Execution {
        message: format!("Failed to read response body: {}", e),
    })?;

    // Try to parse as JSON
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let content = if content_type.contains("application/json") {
        serde_json::from_str(&body_text).unwrap_or(serde_json::json!(body_text))
    } else {
        serde_json::json!(body_text)
    };

    // Build response based on output mode
    let result = if output_mode == "response" {
        let headers_map: serde_json::Map<String, serde_json::Value> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v.to_str().unwrap_or(""))))
            .collect();

        serde_json::json!({
            "request": {
                "method": method.to_uppercase(),
                "uri": url,
                "headers": {}
            },
            "statusCode": status.as_u16(),
            "headers": headers_map,
            "content": content
        })
    } else {
        content
    };

    Ok(result)
}

async fn execute_openapi_v3_spec(
    client: &reqwest::Client,
    task_name: &str,
    operation_id: &str,
    parameters: &serde_json::Value,
    spec: &OpenAPI,
    output_mode: &str,
    doc_endpoint: &str,
) -> Result<serde_json::Value> {
    // Find operation by operationId
    let (path_pattern, method, operation) =
        find_operation(spec, operation_id).ok_or(Error::Execution {
            message: format!("Operation '{}' not found in OpenAPI spec", operation_id),
        })?;

    // Build base URL
    let base_url = if !spec.servers.is_empty() {
        spec.servers.first().map(|s| s.url.as_str()).unwrap_or("")
    } else {
        // Extract from doc endpoint
        let url = Url::parse(doc_endpoint).map_err(|e| Error::Execution {
            message: format!("Failed to parse doc endpoint URL: {}", e),
        })?;
        &format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""))
    };

    // Build URL with path and query parameters
    let mut url = format!("{}{}", base_url, path_pattern);
    let mut query_params = Vec::new();

    // Process parameters
    for param_or_ref in &operation.parameters {
        if let ReferenceOr::Item(param) = param_or_ref {
            let param_name = &param.name;

            if let Some(value) = parameters.get(param_name) {
                let value_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => value.to_string(),
                };

                match &param.kind {
                    ParameterKind::Path { .. } => {
                        url = url.replace(&format!("{{{}}}", param_name), &value_str);
                    }
                    ParameterKind::Query { .. } => {
                        query_params.push(format!("{}={}", param_name, value_str));
                    }
                    _ => {}
                }
            }
        }
    }

    if !query_params.is_empty() {
        url = format!("{}?{}", url, query_params.join("&"));
    }

    println!("  Request: {} {}", method.to_uppercase(), url);

    // Make the HTTP request
    let response = match method.to_uppercase().as_str() {
        "GET" => client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Request failed: {}", e),
            })?,
        "POST" => {
            let body = parameters
                .get("body")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Execution {
                    message: format!("Request failed: {}", e),
                })?
        }
        "PUT" => {
            let body = parameters
                .get("body")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            client
                .put(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Execution {
                    message: format!("Request failed: {}", e),
                })?
        }
        "DELETE" => client
            .delete(&url)
            .send()
            .await
            .map_err(|e| Error::Execution {
                message: format!("Request failed: {}", e),
            })?,
        _ => {
            return Err(Error::Execution {
                message: format!("Unsupported HTTP method: {}", method),
            });
        }
    };

    let status = response.status();
    let headers = response.headers().clone();

    println!("  Response status: {}", status);

    if !status.is_success() {
        let error_obj = serde_json::json!({
            "type": "https://serverlessworkflow.io/dsl/errors/types/communication",
            "status": status.as_u16(),
            "title": format!("HTTP {} Error", status.as_u16()),
            "detail": format!("{} request to {} failed with status {}", method.to_uppercase(), url, status),
            "instance": format!("/do/0/{}", task_name)
        });
        return Err(Error::Execution {
            message: serde_json::to_string(&error_obj).map_err(|e| Error::Execution {
                message: format!("Failed to serialize error: {}", e),
            })?,
        });
    }

    // Get response body
    let body_text = response.text().await.map_err(|e| Error::Execution {
        message: format!("Failed to read response body: {}", e),
    })?;

    // Try to parse as JSON
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let content = if content_type.contains("application/json") {
        serde_json::from_str(&body_text).unwrap_or(serde_json::json!(body_text))
    } else {
        serde_json::json!(body_text)
    };

    // Build response based on output mode
    let result = if output_mode == "response" {
        let headers_map: serde_json::Map<String, serde_json::Value> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v.to_str().unwrap_or(""))))
            .collect();

        serde_json::json!({
            "request": {
                "method": method.to_uppercase(),
                "uri": url,
                "headers": {}
            },
            "statusCode": status.as_u16(),
            "headers": headers_map,
            "content": content
        })
    } else {
        content
    };

    Ok(result)
}

fn find_operation<'a>(
    spec: &'a OpenAPI,
    operation_id: &str,
) -> Option<(&'a str, &'a str, &'a openapiv3::Operation)> {
    for (path, path_item_ref) in &spec.paths.paths {
        if let ReferenceOr::Item(path_item) = path_item_ref {
            if let Some(op) = &path_item.get {
                if op.operation_id.as_deref() == Some(operation_id) {
                    return Some((path.as_str(), "get", op));
                }
            }
            if let Some(op) = &path_item.post {
                if op.operation_id.as_deref() == Some(operation_id) {
                    return Some((path.as_str(), "post", op));
                }
            }
            if let Some(op) = &path_item.put {
                if op.operation_id.as_deref() == Some(operation_id) {
                    return Some((path.as_str(), "put", op));
                }
            }
            if let Some(op) = &path_item.delete {
                if op.operation_id.as_deref() == Some(operation_id) {
                    return Some((path.as_str(), "delete", op));
                }
            }
            if let Some(op) = &path_item.patch {
                if op.operation_id.as_deref() == Some(operation_id) {
                    return Some((path.as_str(), "patch", op));
                }
            }
        }
    }
    None
}

async fn evaluate_parameters(
    parameters: &serde_json::Value,
    ctx: &Context,
) -> Result<serde_json::Value> {
    if let Some(obj) = parameters.as_object() {
        let mut result = serde_json::Map::new();
        let current_data = ctx.data.read().await.clone();

        for (key, value) in obj {
            let evaluated = crate::expressions::evaluate_value_with_input(
                value,
                &current_data,
                &ctx.initial_input,
            )
            .map_err(|e| Error::Execution {
                message: format!("Failed to evaluate parameter '{key}': {e}"),
            })?;
            result.insert(key.clone(), evaluated);
        }

        Ok(serde_json::Value::Object(result))
    } else {
        Ok(parameters.clone())
    }
}
