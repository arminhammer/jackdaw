//! `use jackdaw::prelude::*;` brings in everything an application needs: the engine, the types of tasks and
//! workflows, errors, and the macros.
#[cfg(feature = "redb")]
pub use crate::api::Redb;
#[cfg(feature = "yaml")]
pub use crate::api::{CatalogDef, Workflow};
pub use crate::api::{
    CloudEvent, Condition, Divergence, DurationExt, Either, Engine, EngineBuilder, Error, Event,
    EventStream, ExecutionHandle, Fault, Retry, Status, StepError, SyncWorkflowContext,
    TaskContext, TaskDef, TaskError, TaskOptions, UpdateError, WorkflowContext,
    WorkflowContextView, WorkflowDef, WorkflowError, WorkflowResult, error_types, race,
};
pub use jackdaw_macros::{catalog, task, tasks, workflow, workflow_methods};
pub use schemars::JsonSchema;
