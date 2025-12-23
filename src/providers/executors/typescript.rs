use crate::executor::{Error, Result};
use rustyscript::{Module, Runtime, RuntimeOptions};
use serde_json::Value;
use std::collections::HashMap;

/// ``TypeScriptExecutor`` executes TypeScript functions using embedded Deno runtime via rustyscript
/// Note: Each execution creates a fresh runtime since Runtime contains non-Send types (Rc)
pub struct TypeScriptExecutor;

impl TypeScriptExecutor {
    #[must_use]
    pub fn new() -> Self {
        TypeScriptExecutor
    }

    /// Execute a TypeScript function with the given arguments
    /// ``module_path``: Path to the .ts file containing the function
    /// ``function_name``: Name of the exported function to call
    /// args: Arguments to pass to the function
    ///
    /// Note: This is a synchronous function that internally uses blocking operations
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The Deno runtime fails to initialize
    /// - The TypeScript module file cannot be read
    /// - The module fails to load
    /// - The function call fails or the function does not exist
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
        let mut runtime =
            Runtime::new(RuntimeOptions::default()).map_err(|e| Error::Execution {
                message: format!("Failed to create Deno runtime: {e}"),
            })?;

        // Read the TypeScript module file
        let mut module_content =
            std::fs::read_to_string(module_path).map_err(|e| Error::Execution {
                message: format!("Failed to read TypeScript module {module_path}: {e}"),
            })?;

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
        let module_handle = runtime.load_module(&module).map_err(|e| Error::Execution {
            message: format!("Failed to load TypeScript module: {e}"),
        })?;

        // Call the exported function from the module
        // Since we only have 1 arg (a JSON object), pass it directly
        let arg = match args {
            [single] => single.clone(),
            _ => Value::Array(args.to_vec()),
        };

        // Call the function from the module's exports (not global scope)
        let result: Value = runtime
            .call_function(Some(&module_handle), function_name, &arg)
            .map_err(|e| Error::Execution {
                message: format!("Failed to call TypeScript function {function_name}: {e}",),
            })?;

        Ok(result)
    }

    /// Execute JavaScript/TypeScript script using embedded Deno runtime with stdin, arguments, and environment variables
    async fn exec_script(
        &self,
        script: &str,
        stdin: Option<&str>,
        arguments: Option<&[String]>,
        environment: Option<&HashMap<String, String>>,
    ) -> Result<Value> {
        let script = script.to_string();
        let stdin = stdin.map(String::from);
        let arguments = arguments.map(<[String]>::to_vec);
        let environment = environment.cloned();

        // Use spawn_blocking since rustyscript creates its own Tokio runtime internally
        tokio::task::spawn_blocking(move || {
            // Create a fresh runtime for this execution
            let mut runtime =
                Runtime::new(RuntimeOptions::default()).map_err(|e| Error::Execution {
                    message: format!("Failed to create Deno runtime: {e}"),
                })?;

            // Inject stdin, arguments, and environment as global variables
            // We need to wrap the user's script to capture stdout/stderr
            let mut wrapper_script = String::new();

            // Create a mock stdin object if stdin is provided
            if let Some(stdin_data) = &stdin {
                let stdin_json =
                    serde_json::to_string(stdin_data).map_err(|e| Error::Execution {
                        message: format!("Failed to serialize stdin: {e}"),
                    })?;
                wrapper_script.push_str(&format!(
                    r#"
// Mock stdin with readFileSync-like behavior
const stdinData = {stdin_json};

// Mock process.stdin with fd property
globalThis.process = globalThis.process || {{}};
globalThis.process.stdin = {{
    fd: 0
}};

// Mock fs module for stdin reading
const mockFs = {{
    readFileSync: function(fd, encoding) {{
        if (fd === 0 || fd === globalThis.process.stdin.fd) {{
            return stdinData;
        }}
        throw new Error('readFileSync only supports stdin (fd 0) in this context');
    }}
}};

// Make it available for import
globalThis.__mockFs = mockFs;
"#
                ));
            } else {
                // Even without stdin, set up process.stdin
                wrapper_script.push_str(
                    r#"
// Set up process.stdin even if no stdin data provided
globalThis.process = globalThis.process || {};
globalThis.process.stdin = {
    fd: 0
};
"#,
                );
            }

            // Set up process.argv
            if let Some(args) = &arguments {
                let args_json = serde_json::to_string(args).map_err(|e| Error::Execution {
                    message: format!("Failed to serialize arguments: {e}"),
                })?;
                wrapper_script.push_str(&format!(
                    r#"
// Set up process.argv (deno-style: [path, script, ...args])
globalThis.process = globalThis.process || {{}};
globalThis.process.argv = ['deno', 'script.js', ...{args_json}];
"#
                ));
            } else {
                wrapper_script.push_str(
                    r#"
// Set up empty process.argv
globalThis.process = globalThis.process || {};
globalThis.process.argv = ['deno', 'script.js'];
"#,
                );
            }

            // Set up process.env
            if let Some(env) = &environment {
                let env_json = serde_json::to_string(env).map_err(|e| Error::Execution {
                    message: format!("Failed to serialize environment: {e}"),
                })?;
                wrapper_script.push_str(&format!(
                    r#"
// Set up process.env
globalThis.process.env = globalThis.process.env || {{}};
Object.assign(globalThis.process.env, {env_json});
"#
                ));
            }

            // Capture stdout and stderr
            wrapper_script.push_str(
                r#"
// Capture console output
let capturedStdout = '';
let capturedStderr = '';
const originalLog = console.log;
const originalError = console.error;
const originalWarn = console.warn;

console.log = (...args) => {
    capturedStdout += args.map(a => String(a)).join(' ') + '\n';
};
console.error = (...args) => {
    capturedStderr += args.map(a => String(a)).join(' ') + '\n';
};
console.warn = (...args) => {
    capturedStderr += args.map(a => String(a)).join(' ') + '\n';
};

// Run user script
try {
"#,
            );

            // Add the user's script (properly handle import statements if any)
            // Replace node:fs imports with our mock
            let modified_script = script.replace(
                "import { readFileSync } from 'node:fs'",
                "const { readFileSync } = globalThis.__mockFs || {}",
            );
            wrapper_script.push_str(&modified_script);

            wrapper_script.push_str(
                r#"
} catch (e) {
    console.error = originalError;
    console.error('Script error:', e.message);
    throw e;
} finally {
    // Restore console
    console.log = originalLog;
    console.error = originalError;
    console.warn = originalWarn;
}

// Export just the stdout - try to parse as JSON, fall back to string
export default (() => {
    try {
        return JSON.parse(capturedStdout);
    } catch {
        return capturedStdout;
    }
})();
"#,
            );

            // Create and load the module
            let module = Module::new("script.js", &wrapper_script);
            let module_handle = runtime.load_module(&module).map_err(|e| Error::Execution {
                message: format!("Failed to load script: {e}"),
            })?;

            // Get the default export
            let result: Value =
                runtime
                    .get_value(Some(&module_handle), "default")
                    .map_err(|e| Error::Execution {
                        message: format!("Failed to get script result: {e}"),
                    })?;

            Ok(result)
        })
        .await
        .map_err(|e| Error::Execution {
            message: format!("Task join error: {e}"),
        })?
    }
}

impl Default for TypeScriptExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl crate::executor::Executor for TypeScriptExecutor {
    async fn exec(
        &self,
        _task_name: &str,
        params: &Value,
        _ctx: &crate::context::Context,
        _streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> Result<Value> {
        // Check if this is a module-based call or script-based
        if let Some(module_path) = params.get("module").and_then(|m| m.as_str()) {
            // Module + function call
            let function_name =
                params
                    .get("function")
                    .and_then(|f| f.as_str())
                    .ok_or(Error::Execution {
                        message: "Missing 'function' parameter".to_string(),
                    })?;

            let args = params
                .get("arguments")
                .and_then(|a| a.as_array())
                .map_or(&[] as &[Value], Vec::as_slice);

            // Execute in spawn_blocking since execute_function is blocking
            let module_path = module_path.to_string();
            let function_name = function_name.to_string();
            let args = args.to_vec();

            tokio::task::spawn_blocking(move || {
                let executor = TypeScriptExecutor::new();
                executor.execute_function(&module_path, &function_name, &args)
            })
            .await
            .map_err(|e| Error::Execution {
                message: format!("Task join error: {e}"),
            })?
        } else if let Some(script) = params.get("script").and_then(|s| s.as_str()) {
            // Script mode: inline script with optional stdin, arguments, and environment
            let stdin = params.get("stdin").and_then(|s| s.as_str());
            let arguments = params
                .get("arguments")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                });
            let environment = params
                .get("environment")
                .and_then(|e| e.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<HashMap<_, _>>()
                });

            self.exec_script(script, stdin, arguments.as_deref(), environment.as_ref())
                .await
        } else {
            Err(Error::Execution {
                message:
                    "TypeScript executor requires either 'module' + 'function' or 'script' parameter"
                        .to_string(),
            })
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
