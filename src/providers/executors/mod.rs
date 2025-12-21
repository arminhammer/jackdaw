mod openapi;
#[cfg(feature = "python-embedded")]
mod python;
#[cfg(not(feature = "python-embedded"))]
mod python_ext;
mod rest;
#[cfg(feature = "typescript-embedded")]
mod typescript;
#[cfg(not(feature = "typescript-embedded"))]
mod typescript_ext;

pub use openapi::OpenApiExecutor;
#[cfg(feature = "python-embedded")]
pub use python::PythonExecutor;
#[cfg(not(feature = "python-embedded"))]
pub use python_ext::PythonExtExecutor as PythonExecutor;
pub use rest::RestExecutor;
#[cfg(feature = "typescript-embedded")]
pub use typescript::TypeScriptExecutor;
#[cfg(not(feature = "typescript-embedded"))]
pub use typescript_ext::TypeScriptExtExecutor as TypeScriptExecutor;
