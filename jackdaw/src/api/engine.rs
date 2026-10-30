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
use super::clock::{Clock, SharedClock, SystemClock};
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
use super::workflow::{
    QueryRef, RunTarget, Runnable, SignalRef, UpdateRef, WorkflowDef, WorkflowId,
};
use crate::cache::CacheProvider;
use crate::durableengine::DurableEngine;
use crate::executor::Executor;
use crate::persistence::PersistenceProvider;
use crate::providers::cache::mem::InMemoryCache;
use crate::providers::persistence::InMemoryPersistence;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::future::IntoFuture;
use std::marker::PhantomData;
use std::panic::Location;
use std::sync::Arc;

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
    clock: Option<SharedClock>,
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
            clock: None,
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

    /// The time source for `ctx.now()`, durable sleeps, step timeouts and retry delays. Defaults to [`SystemClock`].
    ///
    /// Tests pass a clock from [`crate::testing`]; sharing one clock between engines keeps time moving across a
    /// simulated crash.
    #[must_use]
    pub fn clock(mut self, clock: impl Clock) -> Self {
        self.clock = Some(Arc::new(clock));
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
                self.clock.unwrap_or_else(|| Arc::new(SystemClock)),
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
        Ok(self.wrap(launched))
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
        self.attach(id, journal).await
    }

    /// A handle to an instance that has a journal: the running instance if it runs here, the stored result if it has
    /// ended, and otherwise the instance resumed from its journal.
    async fn attach<Out>(&self, id: &str, journal: Journal) -> Result<ExecutionHandle<Out>, Error> {
        if let Some(core) = self.shared.instance(id) {
            return Ok(self.wrap(Launched { core }));
        }
        if let Some(ended) = journal.ended() {
            let core = InstanceCore::new(
                id.to_string(),
                journal.workflow.clone(),
                super::workflow::ReplayMode::Strict,
                Arc::downgrade(&self.shared),
                journal,
                None,
                true,
            );
            core.log.close();
            core.publish(ended);
            return Ok(self.wrap(Launched { core }));
        }
        let entry = self.shared.find_workflow(&journal.workflow)?;
        let input = journal.input.clone();
        let launched = self
            .shared
            .launch(id.to_string(), entry, input, journal)
            .await?;
        Ok(self.wrap(launched))
    }

    /// Starts a workflow with [`ExecuteOptions`]: an id of your choosing (see [`execute_with_id`](Self::execute_with_id))
    /// and a deadline for the whole run.
    ///
    /// # Errors
    /// As for [`execute`](Self::execute) and [`execute_with_id`](Self::execute_with_id).
    pub async fn execute_with<R: Runnable>(
        &self,
        runnable: R,
        input: R::In,
        options: ExecuteOptions,
    ) -> Result<ExecutionHandle<R::Out>, Error> {
        let RunTarget::Native(info) = runnable.target() else {
            return Err(Error::invalid(
                "execute_with starts Rust workflows; run YAML workflows with `execute`",
            ));
        };
        let value = serde_json::to_value(&input)
            .map_err(|e| Error::invalid(format!("the input could not be serialized: {e}")))?;
        let _starting = self.shared.starts.lock().await;
        let id = options
            .id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if let Some(journal) = self.shared.load_journal(&id).await? {
            if journal.workflow != info.id {
                return Err(Error::invalid(format!(
                    "instance `{id}` already exists and belongs to workflow {}, not {}",
                    journal.workflow, info.id
                )));
            }
            return self.attach(&id, journal).await;
        }
        let entry = self.shared.find_workflow(&info.id)?;
        let mut journal = Journal::new(info.id.clone(), value.clone());
        if let Some(timeout) = options.timeout {
            journal.deadlines.insert(
                super::runtime::WORKFLOW_DEADLINE.to_string(),
                super::clock::after(self.shared.clock.now(), timeout),
            );
        }
        let launched = self.shared.launch(id, entry, value, journal).await?;
        Ok(self.wrap(launched))
    }

    /// Starts a workflow under an id you choose. Starting the same id again returns the existing instance instead of a
    /// second one, so a retried request cannot start the work twice.
    ///
    /// # Errors
    /// `Error::Invalid` if an instance with this id exists and belongs to a different workflow, or if `runnable` is not
    /// a Rust workflow.
    pub async fn execute_with_id<R: Runnable>(
        &self,
        id: impl Into<String>,
        runnable: R,
        input: R::In,
    ) -> Result<ExecutionHandle<R::Out>, Error> {
        self.execute_with(runnable, input, ExecuteOptions::new().id(id))
            .await
    }

    /// A handle to an existing instance of `W`, for signalling, querying, cancelling or awaiting it. An instance that
    /// is not running in this engine but has not ended is resumed from its journal.
    ///
    /// # Errors
    /// If there is no such instance, or it belongs to a different workflow than `W`.
    pub async fn handle<W: WorkflowDef>(&self, id: &str) -> Result<ExecutionHandle<W::Out>, Error> {
        let journal = self
            .shared
            .load_journal(id)
            .await?
            .ok_or_else(|| Error::Engine(format!("no workflow instance `{id}` was found")))?;
        let wanted = WorkflowId::of::<W>();
        if journal.workflow != wanted {
            return Err(Error::invalid(format!(
                "instance {id} belongs to workflow {}, not {wanted}",
                journal.workflow
            )));
        }
        self.attach(id, journal).await
    }

    /// Resumes every instance in the store that was running when its engine stopped. Call it once after building an
    /// engine over a durable store.
    ///
    /// Child workflows are resumed by their parents. Instances of workflows this engine does not register are left as
    /// they are and reported in [`Recovery::unrecoverable`].
    ///
    /// # Errors
    /// If the store cannot be read.
    pub async fn recover(&self) -> Result<Recovery, Error> {
        let mut recovery = Recovery::default();
        for summary in self.shared.summaries().await? {
            let journal = summary.journal;
            if journal.status != JournalStatus::Running
                || journal.parent.is_some()
                || self.shared.instance(&summary.id).is_some()
            {
                continue;
            }
            match self.shared.find_workflow(&journal.workflow) {
                Ok(entry) => {
                    let input = journal.input.clone();
                    self.shared
                        .launch(summary.id.clone(), entry, input, journal)
                        .await?;
                    recovery.resumed.push(summary.id);
                }
                Err(_) => recovery.unrecoverable.push(Unrecoverable {
                    workflow: journal.workflow.to_string(),
                    reason: format!(
                        "workflow {} is not registered on this engine",
                        journal.workflow
                    ),
                    id: summary.id,
                }),
            }
        }
        Ok(recovery)
    }

    /// The instances in the store that match `filter`, oldest first.
    ///
    /// # Errors
    /// If the store cannot be read.
    pub async fn list(&self, filter: &ListFilter) -> Result<Vec<InstanceInfo>, Error> {
        let mut found: Vec<InstanceInfo> = self
            .shared
            .summaries()
            .await?
            .into_iter()
            .map(|summary| InstanceInfo {
                status: summary.journal.status.into(),
                started_at: summary.journal.created_at,
                updated_at: summary.journal.updated_at,
                parent: summary.journal.parent.clone(),
                workflow: summary.journal.workflow,
                id: summary.id,
            })
            .filter(|info| filter.matches(info))
            .collect();
        found.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
        Ok(found
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit.unwrap_or(usize::MAX))
            .collect())
    }

    /// Cancels an instance running in this engine, and its child workflows.
    pub fn cancel(&self, id: &str) {
        self.shared.cancel(id);
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
        let timeout = options.timeout;
        let mut spec = core.spec(
            "call",
            task.task_name().to_string(),
            value,
            Location::caller(),
            StepExec::Task {
                target: task.target(),
                options,
            },
        );
        spec.timeout = timeout;
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
        for id in self.shared.indexed().await? {
            if let Some(divergence) = replay_instance(&self.shared, &id).await? {
                divergences.push(divergence);
            }
        }
        Ok(divergences)
    }

    fn wrap<Out>(&self, launched: Launched) -> ExecutionHandle<Out> {
        ExecutionHandle {
            shared: Arc::clone(&self.shared),
            core: launched.core,
            _out: PhantomData,
        }
    }
}

/// A running (or finished) workflow instance. `.await` it for the result.
pub struct ExecutionHandle<Out> {
    shared: Arc<Shared>,
    core: Arc<InstanceCore>,
    _out: PhantomData<fn() -> Out>,
}

impl<Out> Clone for ExecutionHandle<Out> {
    /// Another handle to the same instance. Every clone can be awaited for the result.
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            core: Arc::clone(&self.core),
            _out: PhantomData,
        }
    }
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

    /// The same handle with the output as JSON, for keeping handles to different workflows in one collection.
    #[must_use]
    pub fn erase(self) -> ExecutionHandle<Value> {
        ExecutionHandle {
            shared: self.shared,
            core: self.core,
            _out: PhantomData,
        }
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
        self.shared.cancel(&self.core.id);
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

    /// Sends an `#[update]` and returns the handler's answer. The update is recorded like a signal when the handler
    /// accepts it.
    ///
    /// # Errors
    /// `Error::Rejected` if the handler refuses the argument; otherwise as for [`signal`](Self::signal).
    pub async fn update<W, A: Serialize, R: DeserializeOwned>(
        &self,
        update: UpdateRef<W, A, R>,
        argument: A,
    ) -> Result<R, Error> {
        self.send_update(update, argument, None).await
    }

    /// Like [`update`](Self::update), but idempotent: sending the same `update_id` again returns the first result
    /// without running the handler again.
    ///
    /// # Errors
    /// As for [`update`](Self::update).
    pub async fn update_with_id<W, A: Serialize, R: DeserializeOwned>(
        &self,
        update: UpdateRef<W, A, R>,
        argument: A,
        update_id: impl Into<String>,
    ) -> Result<R, Error> {
        self.send_update(update, argument, Some(update_id.into()))
            .await
    }

    async fn send_update<W, A: Serialize, R: DeserializeOwned>(
        &self,
        update: UpdateRef<W, A, R>,
        argument: A,
        id: Option<String>,
    ) -> Result<R, Error> {
        let value = serde_json::to_value(argument).map_err(|e| {
            Error::invalid(format!("the update argument could not be serialized: {e}"))
        })?;
        let result = self.core.deliver_update(update.name, value, id).await?;
        serde_json::from_value(result)
            .map_err(|e| Error::Engine(format!("the update result does not match its type: {e}")))
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
    /// The event is stored with the instance before this returns, so it is not lost if the process stops before the
    /// workflow consumes it.
    ///
    /// # Errors
    /// If the payload cannot be serialized, the instance is not running, or the store cannot be written.
    pub async fn send_event(&self, name: &str, payload: impl Serialize) -> Result<(), Error> {
        let value = serde_json::to_value(payload)
            .map_err(|e| Error::invalid(format!("the event could not be serialized: {e}")))?;
        self.core.deliver_event(name, value).await
    }
}

impl<Out: DeserializeOwned + Send + 'static> IntoFuture for ExecutionHandle<Out> {
    type Output = Result<Out, Error>;
    type IntoFuture = BoxFuture<'static, Result<Out, Error>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _engine = self.shared; // keep the engine alive while someone waits
            let value = self.core.outcome().await?;
            serde_json::from_value(value).map_err(|e| {
                Error::Faulted(super::error::Fault::validation(format!(
                    "the workflow's output does not match its type: {e}"
                )))
            })
        })
    }
}

/// What [`Engine::recover`] did.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Recovery {
    /// Instances that were resumed.
    pub resumed: Vec<String>,
    /// Running instances this engine cannot resume, left untouched in the store.
    pub unrecoverable: Vec<Unrecoverable>,
}

/// A running instance [`Engine::recover`] could not resume.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Unrecoverable {
    /// The instance id.
    pub id: String,
    /// The workflow identity recorded for the instance.
    pub workflow: String,
    /// Why it was not resumed.
    pub reason: String,
}

/// One stored instance, as [`Engine::list`] reports it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InstanceInfo {
    /// The instance id.
    pub id: String,
    /// The workflow it runs.
    pub workflow: WorkflowId,
    /// Where it is in its life.
    pub status: Status,
    /// When it started.
    pub started_at: Option<DateTime<Utc>>,
    /// When its journal last changed.
    pub updated_at: Option<DateTime<Utc>>,
    /// The instance that runs it as a child workflow.
    pub parent: Option<String>,
}

/// Which instances [`Engine::list`] returns. The default lists every top-level instance.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    status: Option<Status>,
    workflow: Option<String>,
    started_after: Option<DateTime<Utc>>,
    started_before: Option<DateTime<Utc>>,
    include_children: bool,
    offset: usize,
    limit: Option<usize>,
}

impl ListFilter {
    /// Every top-level instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Only instances with this status.
    #[must_use]
    pub fn status(mut self, status: Status) -> Self {
        self.status = Some(status);
        self
    }

    /// Only instances of this workflow: `"namespace/name"` matches every version, `"namespace/name/version"` one.
    #[must_use]
    pub fn workflow(mut self, workflow: impl Into<String>) -> Self {
        self.workflow = Some(workflow.into());
        self
    }

    /// Only instances started after this time.
    #[must_use]
    pub fn started_after(mut self, time: DateTime<Utc>) -> Self {
        self.started_after = Some(time);
        self
    }

    /// Only instances started before this time.
    #[must_use]
    pub fn started_before(mut self, time: DateTime<Utc>) -> Self {
        self.started_before = Some(time);
        self
    }

    /// Also list child workflow instances.
    #[must_use]
    pub fn include_children(mut self, include: bool) -> Self {
        self.include_children = include;
        self
    }

    /// Skips the first `offset` matches (for paging).
    #[must_use]
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Returns at most `limit` matches (for paging).
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    fn matches(&self, info: &InstanceInfo) -> bool {
        let workflow_matches = self.workflow.as_ref().is_none_or(|wanted| {
            let name = format!("{}/{}", info.workflow.namespace, info.workflow.name);
            *wanted == name || *wanted == format!("{name}/{}", info.workflow.version)
        });
        (self.include_children || info.parent.is_none())
            && self.status.is_none_or(|status| status == info.status)
            && workflow_matches
            && self
                .started_after
                .is_none_or(|after| info.started_at.is_some_and(|at| at > after))
            && self
                .started_before
                .is_none_or(|before| info.started_at.is_some_and(|at| at < before))
    }
}

/// How [`Engine::execute_with`] starts an instance.
#[derive(Debug, Clone, Default)]
pub struct ExecuteOptions {
    id: Option<String>,
    timeout: Option<std::time::Duration>,
}

impl ExecuteOptions {
    /// No id (a random one is used) and no deadline.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the instance under this id; starting it again returns the existing instance.
    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Ends the instance with `Error::Timeout` if it is still running this long after it started. The deadline is
    /// stored with the instance, so it holds across restarts.
    #[must_use]
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}
