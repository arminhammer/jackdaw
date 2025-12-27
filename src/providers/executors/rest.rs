use crate::context::Context;
use crate::executor::{Error, Executor, Result};
use async_trait::async_trait;

pub struct RestExecutor(pub reqwest::Client);

#[async_trait]
impl Executor for RestExecutor {
    async fn exec(
        &self,
        task_name: &str,
        params: &serde_json::Value,
        ctx: &Context,
        _streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> Result<serde_json::Value> {
        // Extract endpoint - can be a string or an object with 'uri' field
        let (endpoint_str, auth_config) = if let Some(endpoint_val) = params.get("endpoint") {
            match endpoint_val {
                serde_json::Value::String(s) => (s.clone(), None),
                serde_json::Value::Object(obj) => {
                    let uri = obj
                        .get("uri")
                        .and_then(|v| v.as_str())
                        .ok_or(Error::Execution {
                            message: "No 'uri' field in endpoint object".to_string(),
                        })?
                        .to_string();
                    let auth = obj.get("authentication").cloned();
                    (uri, auth)
                }
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::Array(_) => {
                    return Err(Error::Execution {
                        message: "Invalid endpoint format".to_string(),
                    });
                }
            }
        } else {
            return Err(Error::Execution {
                message: "No endpoint specified".to_string(),
            });
        };

        // Interpolate path parameters from context if needed
        let endpoint = interpolate_uri(&endpoint_str, ctx).await?;

        let method = params
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("get");

        // Check for output mode (default is "content")
        let output_mode = params
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("content");

        // Check for redirect setting (default is true - follow redirects)
        let follow_redirects = params
            .get("redirect")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        // Create appropriate client based on redirect policy
        let client = if follow_redirects {
            // Use the default client (which follows redirects by default)
            self.0.clone()
        } else {
            // Create a client that doesn't follow redirects
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| Error::Execution {
                    message: format!("Failed to create HTTP client: {}", e),
                })?
        };

        // Build the request
        let mut request_builder = match method.to_lowercase().as_str() {
            "post" => client.post(&endpoint),
            "put" => client.put(&endpoint),
            "delete" => client.delete(&endpoint),
            "patch" => client.patch(&endpoint),
            _ => client.get(&endpoint),
        };

        // Add authentication if specified
        if let Some(auth) = auth_config {
            request_builder = apply_authentication(request_builder, &auth, ctx).await?;
        }

        // Add body for POST/PUT requests
        if (method == "post" || method == "put" || method == "patch")
            && let Some(body) = params.get("body")
        {
            request_builder = request_builder.json(body);
        }

        // Send the request
        let res = request_builder.send().await;

        match res {
            Ok(response) => {
                let status = response.status();
                let headers = response.headers().clone();

                // Check if the response indicates an error
                // When redirects are disabled, 3xx responses are valid and should be returned
                let is_redirect = status.is_redirection();
                let treat_as_error = !status.is_success() && (!is_redirect || follow_redirects);

                if treat_as_error {
                    // Create a structured error object
                    let error_obj = serde_json::json!({
                        "type": "https://serverlessworkflow.io/dsl/errors/types/communication",
                        "status": status.as_u16(),
                        "title": format!("HTTP {} Error", status.as_u16()),
                        "detail": format!("{} request to {} failed with status {}", method.to_uppercase(), endpoint, status),
                        "instance": format!("/do/0/{}/try/0/{}", ctx.state.current_task.read().await, task_name)
                    });

                    // Return error as JSON string
                    return Err(Error::Execution {
                        message: serde_json::to_string(&error_obj).map_err(|e| {
                            Error::Execution {
                                message: format!("Failed to serialize error: {e}"),
                            }
                        })?,
                    });
                }

                // Get response body
                let body_text = response.text().await.map_err(|e| Error::Execution {
                    message: format!("Failed to read response body: {e}"),
                })?;

                // Try to parse as JSON if content-type is application/json
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
                    // Return full HTTP response
                    let headers_map: serde_json::Map<String, serde_json::Value> = headers
                        .iter()
                        .map(|(k, v)| (k.to_string(), serde_json::json!(v.to_str().unwrap_or(""))))
                        .collect();

                    serde_json::json!({
                        "request": {
                            "method": method.to_uppercase(),
                            "uri": endpoint,
                            "headers": {}
                        },
                        "statusCode": status.as_u16(),
                        "headers": headers_map,
                        "content": content
                    })
                } else {
                    // Return just the content
                    content
                };

                Ok(result)
            }
            Err(e) => {
                // Network or other error
                let error_obj = serde_json::json!({
                    "type": "https://serverlessworkflow.io/dsl/errors/types/communication",
                    "status": 500,
                    "title": "Communication Error",
                    "detail": e.to_string(),
                    "instance": format!("/do/0/{}/try/0/{}", ctx.state.current_task.read().await, task_name)
                });

                Err(Error::Execution {
                    message: serde_json::to_string(&error_obj).map_err(|e| Error::Execution {
                        message: format!("Failed to serialize error: {e}"),
                    })?,
                })
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

async fn apply_authentication(
    request: reqwest::RequestBuilder,
    auth_config: &serde_json::Value,
    ctx: &Context,
) -> Result<reqwest::RequestBuilder> {
    // Handle basic authentication
    if let Some(basic) = auth_config.get("basic") {
        let username = if let Some(username_expr) = basic.get("username") {
            // Evaluate expression
            let current_data = ctx.state.data.read().await.clone();
            let evaluated = crate::expressions::evaluate_value_with_input(
                username_expr,
                &current_data,
                &ctx.metadata.initial_input,
            )
            .map_err(|e| Error::Execution {
                message: format!("Failed to evaluate username expression: {e}"),
            })?;
            evaluated.as_str().unwrap_or("").to_string()
        } else {
            return Err(Error::Execution {
                message: "No username in basic auth".to_string(),
            });
        };

        let password = if let Some(password_expr) = basic.get("password") {
            // Evaluate expression
            let current_data = ctx.state.data.read().await.clone();
            let evaluated = crate::expressions::evaluate_value_with_input(
                password_expr,
                &current_data,
                &ctx.metadata.initial_input,
            )
            .map_err(|e| Error::Execution {
                message: format!("Failed to evaluate password expression: {e}"),
            })?;
            evaluated.as_str().unwrap_or("").to_string()
        } else {
            return Err(Error::Execution {
                message: "No password in basic auth".to_string(),
            });
        };

        Ok(request.basic_auth(username, Some(password)))
    } else {
        // Other auth types not yet implemented
        Ok(request)
    }
}

async fn interpolate_uri(uri: &str, ctx: &Context) -> Result<String> {
    // Simple URI interpolation - replace {paramName} with values from context
    let mut result = uri.to_string();
    let data = ctx.state.data.read().await;

    // Find all {paramName} patterns and replace them
    let re = regex::Regex::new(r"\{([^}]+)\}").map_err(|e| Error::Execution {
        message: format!("Failed to compile regex: {e}"),
    })?;

    for cap in re.captures_iter(uri) {
        let param_name = &cap[1];
        if let Some(value) = data.get(param_name) {
            let value_str = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null
                | serde_json::Value::Array(_)
                | serde_json::Value::Object(_) => value.to_string(),
            };
            result = result.replace(&format!("{{{param_name}}}"), &value_str);
        }
    }

    Ok(result)
}
