//! Typed bindings to catalog entries: tasks and workflows published as spec YAML.
use super::task::{TaskDef, TaskTarget};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Task,
    Workflow,
}

/// What the Rust side claims about a catalog entry: its name and version, and the shapes of its input and output.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub kind: EntryKind,
    pub name: &'static str,
    pub version: &'static str,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}

impl CatalogEntry {
    #[must_use]
    pub fn task<I: JsonSchema, O: JsonSchema>(name: &'static str, version: &'static str) -> Self {
        Self::new::<I, O>(EntryKind::Task, name, version)
    }

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
pub trait CatalogDef {
    /// Where the catalog lives: `file://./path` or `https://host/path`.
    const SOURCE: &'static str;

    fn entries() -> Vec<CatalogEntry>;
}

/// The marker of a catalog task (`Local::add`).
pub struct CatalogTaskRef<I, O> {
    name: &'static str,
    version: &'static str,
    _types: PhantomData<fn(I) -> O>,
}

impl<I, O> CatalogTaskRef<I, O> {
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

/// The marker of a catalog workflow (`Local::normalize`).
pub struct CatalogWorkflowRef<I, O> {
    pub(crate) name: &'static str,
    pub(crate) version: &'static str,
    _types: PhantomData<fn(I) -> O>,
}

impl<I, O> CatalogWorkflowRef<I, O> {
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
