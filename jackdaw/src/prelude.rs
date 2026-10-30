//! `use jackdaw::prelude::*;` brings in everything an application needs: the engine, the types of tasks and
//! workflows, errors, and the macros.
pub use crate::api::{
    CatalogDef, Condition, Divergence, DurationExt, Either, Engine, EngineBuilder, Error, Event,
    EventStream, ExecutionHandle, Fault, Redb, Retry, Status, StepError, SyncWorkflowContext,
    TaskContext, TaskDef, TaskError, TaskOptions, Workflow, WorkflowContext, WorkflowContextView,
    WorkflowDef, WorkflowError, WorkflowResult, error_types, race,
};
pub use jackdaw_macros::{catalog, task, tasks, workflow, workflow_methods};
pub use schemars::JsonSchema;
