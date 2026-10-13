//! Procedural macros for jackdaw. Use them through `jackdaw::prelude`, not by depending on this crate directly.
//!
//! - `#[task]` / `#[tasks]`: typed native tasks (a free function, or the methods of an `impl` that holds state).
//! - `#[workflow]` / `#[workflow_methods]`: a workflow written in Rust, with `#[run]`, `#[init]`, `#[signal]`, `#[query]`.
//! - `#[catalog]`: typed bindings to the tasks and workflows of a catalog.
//!
//! These have to be a separate crate because Rust only lets a `proc-macro` crate export macros.
use proc_macro::TokenStream;

mod args;
mod catalog;
mod scan;
mod task;
mod util;
mod workflow;

fn finish(result: syn::Result<proc_macro2::TokenStream>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Turns an `async fn(input: In) -> Result<Out, TaskError>` into a task: generates the typed marker `CamelCaseName`.
///
/// Arguments: `name = ".."` (default: the function name), `cached`, `blocking` (for a synchronous function).
#[proc_macro_attribute]
pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(task::expand_fn(attr.into(), item.into()))
}

/// On an `impl` block: every `#[task]` method becomes a task and an associated const of the same name.
#[proc_macro_attribute]
pub fn tasks(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(task::expand_impl(attr.into(), item.into()))
}

/// On a struct (the workflow's state): `#[workflow(namespace = "..", name = "..", version = "..", replay = "strict")]`.
#[proc_macro_attribute]
pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(workflow::expand_struct(attr.into(), item.into()))
}

/// On the `impl` of a `#[workflow]` struct: `#[run]`, optional `#[init]`, `#[signal]` and `#[query]` methods.
#[proc_macro_attribute]
pub fn workflow_methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(workflow::expand_methods(attr.into(), item.into()))
}

/// On a trait: declares typed catalog entries, `#[catalog(source = "file://./catalog")]`.
#[proc_macro_attribute]
pub fn catalog(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(catalog::expand(attr.into(), item.into()))
}
