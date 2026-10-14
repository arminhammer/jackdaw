//! `Engine`, its builder, and the handle to a running workflow.
//!
//! ```
//! use jackdaw::prelude::*;
//!
//! #[task]
//! async fn shout(text: String) -> Result<String, TaskError> {
//!     Ok(text.to_uppercase())
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let engine = Engine::builder().task(Shout).build()?;
//! let out = engine.run_task(Shout, "hi".to_string(), TaskOptions::new()).await?;
//! assert_eq!(out, "HI");
//! # Ok(())
//! # }
//! ```
use super::catalog::CatalogDef;
use super::error::{Divergence, Error};
use super::event::{Event, EventStream};
use super::runtime::{
    CatalogSet, InstanceCore, Journal, JournalStatus, Launched, Registry, Shared, StepExec,
    replay_instance,
};
use super::store::Redb;
use super::task::{
    BoxFuture, NativeTask, RegisterTask, TaskContext, TaskDef, TaskOptions, TaskSet,
};
use super::workflow::{QueryRef, RunTarget, Runnable, SignalRef, WorkflowDef, WorkflowId};
use crate::cache::CacheProvider;
use crate::durableengine::DurableEngine;
use crate::executor::Executor;
use crate::persistence::PersistenceProvider;
use crate::providers::cache::mem::InMemoryCache;
use crate::providers::persistence::InMemoryPersistence;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::future::IntoFuture;
use std::marker::PhantomData;
use std::panic::Location;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Where an instance is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Status {
    /// Still executing, or waiting on a timer, signal or event.
    Running,
    /// Finished with a result.
    Completed,
    /// Ended with an error.
    Faulted,
    /// Stopped by [`ExecutionHandle::cancel`].
    Cancelled,
}

impl From<JournalStatus> for Status {
    fn from(status: JournalStatus) -> Self {
        match status {
            JournalStatus::Running => Self::Running,
            JournalStatus::Completed => Self::Completed,
            JournalStatus::Faulted => Self::Faulted,
            JournalStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// Makes `EngineBuilder::tasks` accept either a value or an `Arc` of it.
pub trait IntoTaskSet {
    /// The task set type.
    type Set: TaskSet;

    /// The shared set.
    fn into_task_set(self) -> Arc<Self::Set>;
}

impl<T: TaskSet> IntoTaskSet for Arc<T> {
    type Set = T;

    fn into_task_set(self) -> Arc<T> {
        self
    }
}

impl<T: TaskSet> IntoTaskSet for T {
    type Set = T;

    fn into_task_set(self) -> Arc<T> {
        Arc::new(self)
    }
}

/// Lets YAML workflows `call:` a native task by name.
struct NativeExecutor(NativeTask);

#[async_trait::async_trait]
impl Executor for NativeExecutor {
    async fn exec(
        &self,
        task_name: &str,
        params: &Value,
        ctx: &crate::context::Context,
        _streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> crate::executor::Result<Value> {
        let instance_id = ctx.metadata.instance_id.clone();
        let key = format!("{instance_id}/{task_name}");
        let context = TaskContext::new(&instance_id, self.0.name, 1, key);
        self.0
            .invoke(context, params.clone())
            .await
            .map_err(|error| crate::executor::Error::Task {
                message: error.as_fault().to_string(),
            })
    }

    fn cacheable(&self) -> bool {
        self.0.cached
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Builds an [`Engine`]. Without [`store`](Self::store) everything lives in memory and is lost when the process ends.
///
/// `build` checks the registrations: every task and child workflow a `#[run]` body refers to must be registered, and
/// catalog entries must exist.
///
/// ```no_run
/// use jackdaw::prelude::*;
///
/// # fn main() -> Result<(), Error> {
/// let engine = Engine::builder()
///     .store(Redb::open("jackdaw.redb")?)
///     .build()?;
/// # let _ = engine;
/// # Ok(())
/// # }
/// ```
pub struct EngineBuilder {
    persistence: Option<Arc<dyn PersistenceProvider>>,
    cache: Option<Arc<dyn CacheProvider>>,
    registry: Registry,
    catalogs: CatalogSet,
    event_buffer: usize,
    self_replay: bool,
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            persistence: None,
            cache: None,
            registry: Registry::default(),
            catalogs: CatalogSet::default(),
            event_buffer: 1000,
            self_replay: false,
        }
    }

    /// Persists workflow state and caches results in one durable store.
    #[must_use]
    pub fn store(mut self, store: Redb) -> Self {
        self.persistence = Some(store.persistence());
        self.cache = Some(store.cache());
        self
    }

    /// Uses a custom persistence provider for workflow state.
    #[must_use]
    pub fn persistence(mut self, persistence: Arc<dyn PersistenceProvider>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Uses a custom cache provider for cached task results.
    #[must_use]
    pub fn cache(mut self, cache: Arc<dyn CacheProvider>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Registers a task defined with `#[task]` on a free function.
    #[must_use]
    pub fn task(mut self, task: impl RegisterTask) -> Self {
        self.registry.add_task(task.native());
        self
    }

    /// Registers the `#[task]` methods of a `#[tasks]` impl, bound to the state in `set`.
    #[must_use]
    pub fn tasks(mut self, set: impl IntoTaskSet) -> Self {
        for task in set.into_task_set().native_tasks() {
            self.registry.add_task(task);
        }
        self
    }

    /// Registers a workflow (and, automatically, the child workflows it runs).
    #[must_use]
    pub fn workflow<W: WorkflowDef>(mut self) -> Self {
        self.registry.add_workflow::<W>(true);
        self
    }

    /// Declares a catalog bound with `#[catalog]`.
    #[must_use]
    pub fn catalog<C: CatalogDef>(mut self) -> Self {
        self.catalogs.add::<C>();
        self
    }

    /// Accepted for compatibility: events are kept per instance and never block a run.
    #[must_use]
    pub fn event_buffer_size(mut self, size: usize) -> Self {
        self.event_buffer = size.max(1);
        self
    }

    pub(crate) fn self_replay(mut self, enabled: bool) -> Self {
        self.self_replay = enabled;
        self
    }

    /// Checks the registrations and creates the engine.
    ///
    /// # Errors
    /// `Error::Invalid` if registrations conflict or refer to something that does not exist.
    pub fn build(self) -> Result<Engine, Error> {
        self.registry.validate(&self.catalogs)?;
        self.catalogs.validate()?;

        let persistence = self.persistence.unwrap_or_else(|| {
            Arc::new(InMemoryPersistence::new()) as Arc<dyn PersistenceProvider>
        });
        let cache = self
            .cache
            .unwrap_or_else(|| Arc::new(InMemoryCache::new()) as Arc<dyn CacheProvider>);

        let mut legacy = DurableEngine::new_with_config(
            Arc::clone(&persistence),
            Arc::clone(&cache),
            self.event_buffer,
        )
        .map_err(|e| Error::Engine(e.to_string()))?;
        let natives: HashMap<String, Box<dyn Executor>> = self
            .registry
            .tasks
            .iter()
            .map(|(name, task)| {
                (
                    (*name).to_string(),
                    Box::new(NativeExecutor(task.clone())) as Box<dyn Executor>,
                )
            })
            .collect();
        legacy.add_executors(natives);

        Ok(Engine {
            shared: Arc::new(Shared::new(
                persistence,
                cache,
                self.registry,
                self.catalogs,
                legacy,
                self.self_replay,
            )),
        })
    }
}

/// Runs workflows: Rust workflows written with `#[workflow]`, and YAML workflows bound to Rust types.
///
/// Dropping the last handle to an engine stops its running instances without ending them, so a new engine over the
/// same store can resume them.
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl Engine {
    /// Starts configuring an engine.
    #[must_use]
    pub fn builder() -> EngineBuilder {
        EngineBuilder::new()
    }

    /// Starts a workflow and returns as soon as the instance exists.
    ///
    /// # Errors
    /// `Error::Invalid` if the workflow is not registered or calls something that does not exist.
    pub async fn execute<R: Runnable>(
        &self,
        runnable: R,
        input: R::In,
    ) -> Result<ExecutionHandle<R::Out>, Error> {
        let value = serde_json::to_value(&input)
            .map_err(|e| Error::invalid(format!("the input could not be serialized: {e}")))?;
        let launched = match runnable.target() {
            RunTarget::Native(info) => {
                let entry = self.shared.find_workflow(&info.id)?;
                let id = uuid::Uuid::new_v4().to_string();
                let journal = Journal::new(info.id.clone(), value.clone());
                self.shared.launch(id, entry, value, journal).await?
            }
            RunTarget::Definition(definition) => {
                self.shared.launch_definition(&definition, value).await?
            }
            RunTarget::Catalog { name, version } => {
                let definition = self.shared.catalogs.load(name, version).await?;
                self.shared.launch_definition(&definition, value).await?
            }
        };
        Ok(self.handle(launched))
    }

    /// Continues an instance that was stopped (for example because its process died) from its journal.
    ///
    /// # Errors
    /// `Error::Invalid` if the instance belongs to a different workflow or version than `W`.
    pub async fn resume<W: WorkflowDef>(&self, id: &str) -> Result<ExecutionHandle<W::Out>, Error> {
        let journal = self.shared.load_journal(id).await?.ok_or_else(|| {
            Error::Engine(format!(
                "no workflow instance `{id}` was found in the store"
            ))
        })?;
        let wanted = WorkflowId::of::<W>();
        if journal.workflow != wanted {
            return Err(Error::invalid(format!(
                "instance {id} belongs to workflow {}, not {wanted}; resume it with the workflow version that started it",
                journal.workflow
            )));
        }
        let entry = self.shared.find_workflow(&wanted)?;
        let input = journal.input.clone();
        let launched = self
            .shared
            .launch(id.to_string(), entry, input, journal)
            .await?;
        Ok(self.handle(launched))
    }

    /// Runs one task through the same retry, timeout and fault handling a workflow step gets.
    ///
    /// # Errors
    /// The task's failure as an [`Error`].
    pub async fn run_task<T: TaskDef>(
        &self,
        task: T,
        input: T::In,
        options: TaskOptions,
    ) -> Result<T::Out, Error> {
        let value = serde_json::to_value(&input)
            .map_err(|e| Error::invalid(format!("the input could not be serialized: {e}")))?;
        let workflow = WorkflowId {
            namespace: "jackdaw".to_string(),
            name: "run-task".to_string(),
            version: "1".to_string(),
        };
        let core = InstanceCore::new(
            format!("task-{}", uuid::Uuid::new_v4()),
            workflow.clone(),
            super::workflow::ReplayMode::Strict,
            Arc::downgrade(&self.shared),
            Journal::new(workflow, Value::Null),
            None,
            true,
        );
        let spec = core.spec(
            "call",
            task.task_name().to_string(),
            value,
            Location::caller(),
            StepExec::Task {
                target: task.target(),
                options,
            },
        );
        let output = core.run_step(spec).await?;
        serde_json::from_value(output).map_err(|e| {
            Error::Faulted(super::error::Fault::validation(format!(
                "the task's output does not match its type: {e}"
            )))
        })
    }

    /// Where the instance is in its life.
    ///
    /// # Errors
    /// If the instance is unknown to this engine and its store.
    pub async fn status(&self, id: &str) -> Result<Status, Error> {
        if let Some(core) = self.shared.instance(id) {
            return Ok(core.status().into());
        }
        match self.shared.load_journal(id).await? {
            Some(journal) => Ok(journal.status.into()),
            None => Err(Error::Engine(format!(
                "no workflow instance `{id}` was found"
            ))),
        }
    }

    /// The events recorded for an instance.
    ///
    /// # Errors
    /// If the store cannot be read.
    pub async fn history(&self, id: &str) -> Result<Vec<Event>, Error> {
        Ok(self
            .shared
            .persistence
            .get_events(id)
            .await?
            .into_iter()
            .map(Event::from_stored)
            .collect())
    }

    /// Replays every stored instance this engine knows against its current code and reports the ones whose history
    /// no longer matches. Run it before deploying a change.
    ///
    /// # Errors
    /// If the store cannot be read.
    pub async fn replay_all(&self) -> Result<Vec<Divergence>, Error> {
        let mut divergences = Vec::new();
        for id in self.shared.indexed().await {
            if let Some(divergence) = replay_instance(&self.shared, &id).await? {
                divergences.push(divergence);
            }
        }
        Ok(divergences)
    }

    fn handle<Out>(&self, launched: Launched) -> ExecutionHandle<Out> {
        ExecutionHandle {
            shared: Arc::clone(&self.shared),
            core: launched.core,
            done: launched.done,
            _out: PhantomData,
        }
    }
}

/// A running (or finished) workflow instance. `.await` it for the result.
pub struct ExecutionHandle<Out> {
    shared: Arc<Shared>,
    core: Arc<InstanceCore>,
    done: oneshot::Receiver<Result<Value, Error>>,
    _out: PhantomData<fn() -> Out>,
}

impl<Out> fmt::Debug for ExecutionHandle<Out> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionHandle")
            .field("id", &self.core.id)
            .finish_non_exhaustive()
    }
}

impl<Out> ExecutionHandle<Out> {
    /// The instance id, for [`Engine::status`], [`Engine::history`] and [`Engine::resume`].
    #[must_use]
    pub fn id(&self) -> &str {
        &self.core.id
    }

    /// A stream of this run's events from the beginning. Call it as often as you like.
    #[must_use]
    pub fn events(&self) -> EventStream {
        EventStream::new(Arc::clone(&self.core.log))
    }

    /// Stops the workflow. Awaiting it afterwards gives `Error::Cancelled`.
    ///
    /// # Errors
    /// Never, currently; reserved for engines that cancel remotely.
    pub async fn cancel(&self) -> Result<(), Error> {
        self.core.request_cancel();
        Ok(())
    }

    /// Delivers a signal to the workflow.
    ///
    /// # Errors
    /// If the argument does not match the signal's type, or the workflow has no such signal.
    pub async fn signal<W, A: Serialize>(
        &self,
        signal: SignalRef<W, A>,
        argument: A,
    ) -> Result<(), Error> {
        let value = serde_json::to_value(argument).map_err(|e| {
            Error::invalid(format!("the signal argument could not be serialized: {e}"))
        })?;
        self.core.deliver_signal(signal.name, value).await
    }

    /// Reads the workflow's state through a `#[query]` method.
    ///
    /// # Errors
    /// If the workflow has no such query.
    pub async fn query<W, R: DeserializeOwned>(&self, query: QueryRef<W, R>) -> Result<R, Error> {
        let value = self.core.query(query.name).await?;
        serde_json::from_value(value)
            .map_err(|e| Error::Engine(format!("the query result does not match its type: {e}")))
    }

    /// Sends an event that a `ctx.wait_for_event(name)` is waiting for (or will wait for).
    ///
    /// # Errors
    /// If the payload cannot be serialized.
    pub async fn send_event(&self, name: &str, payload: impl Serialize) -> Result<(), Error> {
        let value = serde_json::to_value(payload)
            .map_err(|e| Error::invalid(format!("the event could not be serialized: {e}")))?;
        self.core.deliver_event(name, value);
        Ok(())
    }
}

impl<Out: DeserializeOwned + Send + 'static> IntoFuture for ExecutionHandle<Out> {
    type Output = Result<Out, Error>;
    type IntoFuture = BoxFuture<'static, Result<Out, Error>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _engine = self.shared; // keep the engine alive while someone waits
            let value = self.done.await.map_err(|_| {
                Error::Engine("the engine stopped before the workflow finished".to_string())
            })??;
            serde_json::from_value(value).map_err(|e| {
                Error::Faulted(super::error::Fault::validation(format!(
                    "the workflow's output does not match its type: {e}"
                )))
            })
        })
    }
}
