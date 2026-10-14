//! Procedural macros for jackdaw. Use them through `jackdaw::prelude`, not by depending on this crate directly.
//!
//! - `#[task]` / `#[tasks]`: typed native tasks (a free function, or the methods of an `impl` that holds state).
//! - `#[workflow]` / `#[workflow_methods]`: a workflow written in Rust, with `#[run]`, `#[init]`, `#[signal]`, `#[query]`.
//! - `#[catalog]`: typed bindings to the tasks and workflows of a catalog.
//!
//! These have to be a separate crate because Rust only lets a `proc-macro` crate export macros.
//!
//! The `jackdaw` crate's guide (`jackdaw::guide`) shows each of them in context, with runnable examples.
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
/// The function is kept as written. It takes its input, optionally preceded by a `TaskContext`:
/// `fn(input: In)` or `fn(ctx: TaskContext, input: In)`.
///
/// Arguments: `name = ".."` (default: the function name), `cached` (reuse results for equal input; off by default
/// because tasks are side effects), `blocking` (a synchronous function, run on tokio's blocking pool).
///
/// ```ignore
/// #[task]
/// async fn charge(order: Order) -> Result<Receipt, TaskError> { /* .. */ }
///
/// // `Charge` is the marker: Engine::builder().task(Charge), ctx.call(Charge, &order)
/// ```
#[proc_macro_attribute]
pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(task::expand_fn(attr.into(), item.into()))
}

/// On an `impl` block: every `#[task]` method (taking `self: Arc<Self>`) becomes a task, and an associated const of
/// the same name is its marker (`Gateway::quote`). Register the instance with `EngineBuilder::tasks`.
#[proc_macro_attribute]
pub fn tasks(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(task::expand_impl(attr.into(), item.into()))
}

/// On a struct (the workflow's state): `#[workflow(namespace = "..", name = "..", version = "..", replay = "strict")]`.
///
/// The three names identify the workflow; `replay` is `"strict"` (default) or `"lenient"`. A unit struct needs no
/// constructor; otherwise give the `impl` an `#[init]` method.
#[proc_macro_attribute]
pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(workflow::expand_struct(attr.into(), item.into()))
}

/// On the `impl` of a `#[workflow]` struct: `#[run]`, optional `#[init]`, `#[signal]` and `#[query]` methods.
///
/// Each method becomes an associated const marker of the same name (`Order::run`, `Order::approve`) that carries its
/// types. The `#[run]` body is scanned at compile time, which has two effects: calls that would break replay
/// (`SystemTime::now()`, `tokio::spawn`, ...) are errors, and the tasks and workflows it uses are checked when the
/// engine is built. Silence the first for one statement with `#[allow_nondeterminism]`.
#[proc_macro_attribute]
pub fn workflow_methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(workflow::expand_methods(attr.into(), item.into()))
}

/// On a trait: declares typed catalog entries, `#[catalog(source = "file://./catalog")]`.
///
/// Each method is `#[task(name = "..", version = "..")]` or `#[workflow(name = "..", version = "..")]` with the Rust
/// input and output types in its signature (which must implement `JsonSchema`). The trait name becomes a marker type
/// whose associated consts (`Local::add`) are used like local tasks and workflows.
#[proc_macro_attribute]
pub fn catalog(attr: TokenStream, item: TokenStream) -> TokenStream {
    finish(catalog::expand(attr.into(), item.into()))
}
