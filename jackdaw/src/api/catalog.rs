//! Typed bindings to catalog entries: tasks and workflows published as spec YAML.
//!
//! You normally do not use the types here directly. Declare a catalog with the [`catalog`](crate::prelude::catalog)
//! attribute and register it with [`EngineBuilder::catalog`](crate::api::EngineBuilder::catalog); the guide chapter on
//! [catalogs](crate::guide::catalogs) shows the whole flow.
use super::task::{TaskDef, TaskTarget};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;

/// Whether a catalog entry is called like a task or run like a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Used with `ctx.call(..)`.
    Task,
    /// Used with `ctx.run(..)`.
    Workflow,
}

/// What the Rust side claims about a catalog entry: its name and version, and the shapes of its input and output.
///
/// When the engine is built, each entry is checked against the catalog: the file must exist, and the entry's input
/// schema must not require anything the Rust input type lacks.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    /// Task or workflow.
    pub kind: EntryKind,
    /// The entry's name in the catalog.
    pub name: &'static str,
    /// The entry's version in the catalog.
    pub version: &'static str,
    /// The JSON schema of the Rust input type.
    pub input_schema: serde_json::Value,
    /// The JSON schema of the Rust output type.
    pub output_schema: serde_json::Value,
}

impl CatalogEntry {
    /// An entry that is called as a task, with the schemas of its Rust input and output types.
    #[must_use]
    pub fn task<I: JsonSchema, O: JsonSchema>(name: &'static str, version: &'static str) -> Self {
        Self::new::<I, O>(EntryKind::Task, name, version)
    }

    /// An entry that is run as a workflow, with the schemas of its Rust input and output types.
    #[must_use]
    pub fn workflow<I: JsonSchema, O: JsonSchema>(
        name: &'static str,
        version: &'static str,
    ) -> Self {
        Self::new::<I, O>(EntryKind::Workflow, name, version)
    }

    fn new<I: JsonSchema, O: JsonSchema>(
        kind: EntryKind,
        name: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            kind,
            name,
            version,
            input_schema: serde_json::to_value(schemars::schema_for!(I))
                .unwrap_or(serde_json::Value::Null),
            output_schema: serde_json::to_value(schemars::schema_for!(O))
                .unwrap_or(serde_json::Value::Null),
        }
    }
}

/// A catalog declared with `#[catalog(source = "..")] trait Name { .. }`.
///
/// The attribute implements this trait for a generated unit struct named after the trait, whose associated constants
/// are the typed markers you pass to `ctx.call` and `ctx.run`.
pub trait CatalogDef {
    /// Where the catalog lives: `file://./path` or `https://host/path`.
    const SOURCE: &'static str;

    /// The entries the Rust side uses, with their types.
    fn entries() -> Vec<CatalogEntry>;
}

/// The marker of a catalog task (`Local::add`): its name, version and typed input and output.
///
/// Created by the [`catalog`](crate::prelude::catalog) attribute; pass it to `ctx.call(..)` or
/// [`Engine::run_task`](crate::api::Engine::run_task).
pub struct CatalogTaskRef<I, O> {
    name: &'static str,
    version: &'static str,
    _types: PhantomData<fn(I) -> O>,
}

impl<I, O> CatalogTaskRef<I, O> {
    /// A reference to the catalog task `name` at `version`.
    #[must_use]
    pub const fn new(name: &'static str, version: &'static str) -> Self {
        Self {
            name,
            version,
            _types: PhantomData,
        }
    }
}

impl<I, O> Clone for CatalogTaskRef<I, O> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I, O> Copy for CatalogTaskRef<I, O> {}

impl<I, O> TaskDef for CatalogTaskRef<I, O>
where
    I: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Send + 'static,
{
    type In = I;
    type Out = O;

    fn task_name(&self) -> &'static str {
        self.name
    }

    fn target(&self) -> TaskTarget {
        TaskTarget::Catalog {
            name: self.name,
            version: self.version,
        }
    }
}

/// The marker of a catalog workflow (`Local::normalize`): its name, version and typed input and output.
///
/// Created by the [`catalog`](crate::prelude::catalog) attribute; pass it to `ctx.run(..)`.
pub struct CatalogWorkflowRef<I, O> {
    pub(crate) name: &'static str,
    pub(crate) version: &'static str,
    _types: PhantomData<fn(I) -> O>,
}

impl<I, O> CatalogWorkflowRef<I, O> {
    /// A reference to the catalog workflow `name` at `version`.
    #[must_use]
    pub const fn new(name: &'static str, version: &'static str) -> Self {
        Self {
            name,
            version,
            _types: PhantomData,
        }
    }
}

impl<I, O> Clone for CatalogWorkflowRef<I, O> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I, O> Copy for CatalogWorkflowRef<I, O> {}
