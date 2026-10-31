//! The library API: tasks, workflows, catalogs and the engine that runs them.
//!
//! Most code only needs [`crate::prelude`]. Code the macros (`#[task]`, `#[workflow]`, ...) generate also refers to a few
//! items that are not part of this API; they live in a hidden `jackdaw::__private` module.
#![warn(missing_docs)]

#[cfg(feature = "yaml")]
mod catalog;
mod clock;
mod codec;
mod context;
mod duration;
mod engine;
mod error;
mod event;
mod executor;
mod helpers;
mod observe;
mod runtime;
mod schedule;
mod store;
mod task;
#[cfg(feature = "yaml")]
mod validate;
mod workflow;

#[cfg(feature = "yaml")]
pub use catalog::{CatalogDef, CatalogTaskRef, CatalogWorkflowRef};
pub use clock::{Clock, SystemClock};
pub use codec::PayloadCodec;
pub use context::{Branch, Condition, Either, ForEach, Step, WorkflowContext, race};
pub use duration::DurationExt;
#[cfg(feature = "yaml")]
pub use engine::set_yaml_trace;
pub use engine::{
    Engine, EngineBuilder, ExecuteOptions, ExecutionHandle, InstanceDetails, InstanceInfo,
    IntoTaskSet, ListFilter, PendingTimer, Recovery, Status, Unrecoverable,
};
pub use error::{
    Divergence, Error, Fault, StepError, TaskError, UpdateError, WorkflowError, WorkflowResult,
    error_types,
};
pub use event::{CloudEvent, Event, EventStream};
pub use executor::{Call, Executor};
#[cfg(feature = "docker")]
pub use helpers::Container;
pub use helpers::{ProcessOutput, Script, Shell};
#[cfg(feature = "http-client")]
pub use helpers::{Request, Response};
pub use schedule::{CatchUp, Cron, Overlap, Schedule};
#[cfg(feature = "postgres")]
pub use store::Postgres;
#[cfg(feature = "redb")]
pub use store::Redb;
#[cfg(feature = "sqlite")]
pub use store::Sqlite;
pub use store::Store;
pub use task::{BoxFuture, Retry, TaskContext, TaskDef, TaskOptions, TaskTarget};
#[cfg(feature = "yaml")]
pub use validate::{Issue, Severity, Validation};
#[cfg(feature = "yaml")]
pub use workflow::Workflow;
pub use workflow::{
    QueryRef, RunTarget, Runnable, SignalRef, SyncWorkflowContext, UpdateRef, WorkflowContextView,
    WorkflowDef, WorkflowId, WorkflowInfo, WorkflowRef,
};

/// What the macros' generated code refers to. Re-exported at `jackdaw::__private`; not part of the API.
pub(crate) mod plumbing {
    #[cfg(feature = "yaml")]
    pub use super::catalog::{CatalogEntry, EntryKind};
    pub use super::clock::after as clock_after;
    pub use super::context::CoreRef;
    pub use super::runtime::{Registry, register_workflow};
    pub use super::task::{BoxFuture, NativeTask, RegisterTask, TaskRef, TaskSet};
    pub use super::workflow::{
        AsDependency, Dependency, QueryHandler, ReplayMode, SignalHandler, UpdateHandler,
        WorkflowMeta,
    };
}
