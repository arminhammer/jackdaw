use serverless_workflow_core::models::workflow::WorkflowDefinition;
use snafu::prelude::*;
use std::collections::HashMap;

use crate::context::Context;

use super::{DurableEngine, Error, IoSnafu, Result};

impl DurableEngine {
    /// Try to load and execute a function from a catalog
    ///
    /// Supports three formats:
    /// 1. "function-name:version" - lookup in catalog
    /// 2. "https://..." - direct URL to function.yaml
    /// 3. "file://..." - direct file path to function.yaml
    ///
    /// Returns None if the function reference is not a catalog function
    pub(super) async fn try_load_catalog_function(
        &self,
        function_name: &str,
        with_params: &HashMap<String, serde_json::Value>,
        ctx: &Context,
    ) -> Result<Option<serde_json::Value>> {
        // Parse the function reference to determine if it's a catalog function
        let function_url = if function_name.starts_with("http://")
            || function_name.starts_with("https://")
        {
            // Direct HTTP(S) URL
            function_name.to_string()
        } else if function_name.starts_with("file://") {
            // Direct file URL
            function_name.to_string()
        } else if function_name.contains(':') {
            // Catalog reference: "function-name:version"
            let parts: Vec<&str> = function_name.split(':').collect();
            if parts.len() != 2 {
                return Err(Error::Configuration {
                    message: format!("Invalid catalog function reference: {function_name}"),
                });
            }
            let (name, version) = (parts[0], parts[1]);

            // Look up in catalogs
            let Some(catalogs) = ctx
                .metadata
                .workflow
                .use_
                .as_ref()
                .and_then(|use_| use_.catalogs.as_ref())
            else {
                return Ok(None); // No catalogs defined
            };

            // Try to find in any catalog
            // Use first catalog for now
            let function_url = if let Some(catalog) = catalogs.values().next() {
                // Extract URI from the endpoint enum
                use serverless_workflow_core::models::resource::OneOfEndpointDefinitionOrUri;
                let catalog_uri = match &catalog.endpoint {
                    OneOfEndpointDefinitionOrUri::Uri(uri) => uri.as_str(),
                    OneOfEndpointDefinitionOrUri::Endpoint(endpoint_def) => &endpoint_def.uri,
                };

                // Build function URL based on catalog structure
                let url = if catalog_uri.starts_with("file://") {
                    let base_path = catalog_uri.strip_prefix("file://").ok_or_else(|| {
                        Error::Configuration {
                            message: format!("Invalid file:// URI: {catalog_uri}"),
                        }
                    })?;
                    format!("file://{base_path}/{name}/{version}/function.yaml")
                } else if catalog_uri.starts_with("http://") || catalog_uri.starts_with("https://")
                {
                    // For HTTP catalogs, follow the structure: {catalog}/functions/{name}/{version}/function.yaml
                    format!(
                        "{}/functions/{name}/{version}/function.yaml",
                        catalog_uri.trim_end_matches('/'),
                    )
                } else {
                    return Err(Error::Configuration {
                        message: format!("Unsupported catalog URI scheme: {catalog_uri}"),
                    });
                };

                Some(url)
            } else {
                None
            };

            match function_url {
                Some(url) => url,
                None => return Ok(None), // Not found in catalogs
            }
        } else {
            // Not a catalog function reference
            return Ok(None);
        };

        // Load the function definition
        let function_content = if function_url.starts_with("file://") {
            // Local file
            let path = function_url.strip_prefix("file://").unwrap();
            tokio::fs::read_to_string(path).await.context(IoSnafu)?
        } else {
            // HTTP(S) URL
            let response = reqwest::get(&function_url)
                .await
                .map_err(|e| Error::TaskExecution {
                    message: format!("Failed to fetch catalog function from {function_url}: {e}"),
                })?;

            if !response.status().is_success() {
                return Err(Error::TaskExecution {
                    message: format!(
                        "Failed to fetch catalog function from {}: HTTP {}",
                        function_url,
                        response.status()
                    ),
                });
            }

            response.text().await.map_err(|e| Error::TaskExecution {
                message: format!(
                    "Failed to read catalog function response from {function_url}: {e}"
                ),
            })?
        };

        // Parse the workflow definition
        let function_workflow: WorkflowDefinition = serde_yaml::from_str(&function_content)
            .map_err(|e| Error::Configuration {
                message: format!("Failed to parse catalog function {function_name}: {e}"),
            })?;

        // Execute the catalog function as a nested workflow with the provided inputs
        let input_data = serde_json::to_value(with_params)?;

        // Run the nested workflow (use Box::pin to avoid infinite-sized future)
        let result = Box::pin(self.run_instance(function_workflow, None, input_data)).await?;

        Ok(Some(result))
    }
}
