// What needs a child process or an HTTP client is left out on wasm, which has neither.
#[cfg(all(feature = "mcp", not(target_family = "wasm")))]
mod mcp;
#[cfg(not(target_family = "wasm"))]
mod node;
#[cfg(feature = "openapi")]
mod openapi;
#[cfg(not(target_family = "wasm"))]
mod python;
#[cfg(not(target_family = "wasm"))]
mod rest;

#[cfg(all(feature = "mcp", not(target_family = "wasm")))]
pub use mcp::McpExecutor;
#[cfg(not(target_family = "wasm"))]
pub use node::NodeExecutor as TypeScriptExecutor;
#[cfg(feature = "openapi")]
pub use openapi::OpenApiExecutor;
#[cfg(not(target_family = "wasm"))]
pub use python::PythonExtExecutor as PythonExecutor;
#[cfg(not(target_family = "wasm"))]
pub use rest::RestExecutor;
