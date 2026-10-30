//! The library API: tasks, workflows, catalogs and the engine that runs them.
//!
//! Most code only needs [`crate::prelude`]. The macros (`#[task]`, `#[workflow]`, ...) generate code that refers to the
//! items re-exported here, which is why they are public.
#![warn(missing_docs)]

mod catalog;
mod clock;
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
#[doc(hidden)]
pub use clock::after as clock_after;
pub use clock::{Clock, SystemClock};
pub use context::{Branch, Condition, CoreRef, Either, ForEach, Step, WorkflowContext, race};
pub use duration::DurationExt;
pub use engine::{
    Engine, EngineBuilder, ExecutionHandle, InstanceInfo, IntoTaskSet, ListFilter, Recovery,
    Status, Unrecoverable,
};
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
