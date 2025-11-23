use rustyscript::{Module, Runtime, RuntimeOptions};
use serde_json::Value;
use std::path::Path;
use crate::executor::{Result, Error};

/// TypeScriptExecutor executes TypeScript functions using embedded Deno runtime via rustyscript
/// Note: Each execution creates a fresh runtime since Runtime contains non-Send types (Rc)
pub struct TypeScriptExecutor;

impl TypeScriptExecutor {
    pub fn new() -> Self {
        TypeScriptExecutor
    }

    /// Execute a TypeScript function with the given arguments
    /// module_path: Path to the .ts file containing the function
    /// function_name: Name of the exported function to call
    /// args: Arguments to pass to the function
    ///
    /// Note: This is a synchronous function that internally uses blocking operations
    pub fn execute_function(
        &self,
        module_path: &str,
        function_name: &str,
        args: &[Value],
    ) -> Result<Value> {
        // NOTE: rustyscript creates a Tokio runtime internally, which will panic
        // if called from within an async context. The caller must ensure this is
        // called from spawn_blocking or a non-async context.

        // Create a fresh runtime for this execution
        let mut runtime = Runtime::new(RuntimeOptions::default())
            .map_err(|e| Error::Execution { message: format!("Failed to create Deno runtime: {}", e) })?;

        // Read the TypeScript module file
        let mut module_content = std::fs::read_to_string(module_path)
            .map_err(|e| Error::Execution { message: format!("Failed to read TypeScript module {}: {}", module_path, e) })?;

        // Remove type-only imports as they are stripped at runtime anyway
        // This is a simple workaround for rustyscript 0.12.3's limited import support
        module_content = module_content
            .lines()
            .filter(|line| !line.trim().starts_with("import type"))
            .collect::<Vec<_>>()
            .join("\n");

        // Create a module
        let module = Module::new(module_path, &module_content);

        // Load the module
        let module_handle = runtime
            .load_module(&module)
            .map_err(|e| Error::Execution { message: format!("Failed to load TypeScript module: {}", e) })?;

        // Call the exported function from the module
        // Since we only have 1 arg (a JSON object), pass it directly
        let arg = if args.len() == 1 {
            args[0].clone()
        } else {
            Value::Array(args.to_vec())
        };

        // Call the function from the module's exports (not global scope)
        let result: Value = runtime
            .call_function(Some(&module_handle), function_name, &arg)
            .map_err(|e| {
                Error::Execution {
                    message: format!(
                        "Failed to call TypeScript function {}: {}",
                        function_name,
                        e
                    )
                }
            })?;

        Ok(result)
    }
}

impl Default for TypeScriptExecutor {
    fn default() -> Self {
        Self::new()
    }
}
