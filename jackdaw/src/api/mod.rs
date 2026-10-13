//! The library API: tasks, workflows, catalogs and the engine that runs them.
//!
//! Most code only needs [`crate::prelude`]. The macros (`#[task]`, `#[workflow]`, ...) generate code that refers to the
//! items re-exported here, which is why they are public.
mod catalog;
mod context;
mod duration;
mod engine;
mod error;
mod event;
mod runtime;
mod store;
mod task;
mod workflow;

pub use catalog::{CatalogDef, CatalogEntry, CatalogTaskRef, CatalogWorkflowRef, EntryKind};
pub use context::{Branch, CoreRef, Either, ForEach, Step, WorkflowContext, race};
pub use duration::DurationExt;
pub use engine::{Engine, EngineBuilder, ExecutionHandle, IntoTaskSet, Status};
pub use error::{
    Divergence, Error, Fault, StepError, TaskError, WorkflowError, WorkflowResult, error_types,
};
pub use event::{Event, EventStream};
pub use runtime::{Registry, register_workflow};
pub use store::Redb;
pub use task::{
    BoxFuture, NativeTask, RegisterTask, Retry, TaskContext, TaskDef, TaskOptions, TaskRef,
    TaskSet, TaskTarget,
};
pub use workflow::{
    AsDependency, Dependency, QueryHandler, QueryRef, ReplayMode, RunTarget, Runnable,
    SignalHandler, SignalRef, SyncWorkflowContext, Workflow, WorkflowContextView, WorkflowDef,
    WorkflowId, WorkflowInfo, WorkflowMeta, WorkflowRef,
};
