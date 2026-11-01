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
#[cfg(feature = "yaml")]
use super::catalog::CatalogDef;
use super::clock::{Clock, SharedClock, SystemClock};
use super::codec::PayloadCodec;
use super::error::{Divergence, Error};
use super::event::{Event, EventStream};
#[cfg(feature = "yaml")]
use super::runtime::CatalogSet;
use super::runtime::{
    InstanceCore, Journal, JournalStatus, Launched, Registry, Shared, StepExec, YamlRuntime,
    replay_instance,
};
use super::store::Store;
use super::task::{BoxFuture, RegisterTask, TaskDef, TaskOptions, TaskSet};
#[cfg(feature = "yaml")]
use super::task::{NativeTask, TaskContext};
use super::workflow::{
    QueryRef, RunTarget, Runnable, SignalRef, UpdateRef, WorkflowDef, WorkflowId,
};
use crate::cache::CacheProvider;
#[cfg(feature = "yaml")]
use crate::durableengine::DurableEngine;
#[cfg(feature = "yaml")]
use crate::executor::Executor;
use crate::persistence::PersistenceProvider;
use crate::providers::cache::mem::InMemoryCache;
use crate::providers::persistence::InMemoryPersistence;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
#[cfg(feature = "yaml")]
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
#[cfg(feature = "yaml")]
struct NativeExecutor(NativeTask);

#[cfg(feature = "yaml")]
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
}

/// Lets YAML workflows `call:` a custom call type.
#[cfg(feature = "yaml")]
struct CustomExecutor {
    name: String,
    executor: Arc<dyn super::executor::Executor>,
}

#[cfg(feature = "yaml")]
#[async_trait::async_trait]
impl Executor for CustomExecutor {
    async fn exec(
        &self,
        task_name: &str,
        params: &Value,
        ctx: &crate::context::Context,
        _streamer: Option<crate::task_output::TaskOutputStreamer>,
    ) -> crate::executor::Result<Value> {
        let call = super::executor::Call {
            call_type: self.name.clone(),
            task_name: task_name.to_string(),
            arguments: params.clone(),
            instance_id: ctx.metadata.instance_id.clone(),
        };
        self.executor
            .exec(call)
            .await
            .map_err(|error| crate::executor::Error::Task {
                message: error.as_fault().to_string(),
            })
    }

    fn cacheable(&self) -> bool {
        self.executor.cacheable()
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
    #[cfg(feature = "yaml")]
    catalogs: CatalogSet,
    // Sizes the YAML interpreter's event buffer.
    #[cfg_attr(not(feature = "yaml"), allow(dead_code))]
    event_buffer: usize,
    self_replay: bool,
    clock: Option<SharedClock>,
    codec: Option<Arc<dyn PayloadCodec>>,
    retention: Option<std::time::Duration>,
    schedules: Vec<super::schedule::Schedule>,
    lease: Option<std::time::Duration>,
    engine_id: Option<String>,
    #[cfg(feature = "yaml")]
    yaml_workflows: Vec<crate::dsl::workflow::WorkflowDefinition>,
    poll: std::time::Duration,
    #[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
    listeners: Vec<super::listener::Listener>,
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
            #[cfg(feature = "yaml")]
            catalogs: CatalogSet::default(),
            event_buffer: 1000,
            self_replay: false,
            clock: None,
            codec: None,
            retention: None,
            schedules: Vec::new(),
            lease: None,
            engine_id: None,
            #[cfg(feature = "yaml")]
            yaml_workflows: Vec::new(),
            poll: std::time::Duration::from_secs(1),
            #[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
            listeners: Vec::new(),
        }
    }

    /// Persists workflow state and caches results in one durable store: [`Redb`](super::Redb),
    /// [`Sqlite`](super::Sqlite), [`Postgres`](super::Postgres), or your own [`Store`].
    // The stores are behind features; docs.rs builds with all of them.
    #[cfg_attr(
        not(all(feature = "redb", feature = "sqlite", feature = "postgres")),
        allow(rustdoc::broken_intra_doc_links)
    )]
    #[must_use]
    pub fn store(mut self, store: impl Store) -> Self {
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

    /// Registers a YAML workflow that other YAML workflows run by name (`run: workflow: { namespace, name, version }`).
    /// Workflows you execute directly need no registration.
    #[must_use]
    #[cfg(feature = "yaml")]
    #[cfg_attr(docsrs, doc(cfg(feature = "yaml")))]
    pub fn yaml_workflow<I, O>(mut self, workflow: &super::workflow::Workflow<I, O>) -> Self {
        self.yaml_workflows.push(workflow.definition().clone());
        self
    }

    /// Shares the store with other engines (other processes, usually): each instance is run by the engine that holds
    /// its lease, which lasts `ttl` and is renewed while the engine runs the instance. An engine that stops (or stalls
    /// longer than `ttl`) loses its leases, and another engine's [`Engine::recover`] takes the instances over;
    /// the stalled engine's writes are then rejected. Signals sent through an engine that does not run the instance
    /// reach it through the store.
    ///
    /// YAML instances are leased too, but the YAML interpreter writes to the store directly: an engine that lost a
    /// YAML instance's lease stops it at its next renewal (see [`poll_interval`](Self::poll_interval)) rather than
    /// having its writes rejected.
    ///
    /// The store must support leases (all built-in stores do). Without this setting an engine assumes it is the only
    /// one on its store.
    #[must_use]
    pub fn lease(mut self, ttl: std::time::Duration) -> Self {
        self.lease = Some(ttl);
        self
    }

    /// This engine's id among the engines sharing a store (a random one by default). Leases are held under it.
    #[must_use]
    pub fn engine_id(mut self, id: impl Into<String>) -> Self {
        self.engine_id = Some(id.into());
        self
    }

    /// How often an engine with [`lease`](Self::lease) on looks for signals sent through other engines and renews
    /// its leases (default 1 second).
    #[must_use]
    pub fn poll_interval(mut self, interval: std::time::Duration) -> Self {
        self.poll = interval;
        self
    }

    /// Registers a call type of your own: YAML workflows `call: <name>` it like a built-in protocol.
    ///
    /// The build fails with `Error::Invalid` if `name` is registered twice or is a built-in call type.
    #[must_use]
    pub fn executor(
        mut self,
        name: impl Into<String>,
        executor: impl super::executor::Executor,
    ) -> Self {
        self.registry.add_executor(name.into(), Arc::new(executor));
        self
    }

    /// A test fake that takes the place of the task with the same name.
    pub(crate) fn fake_task(mut self, task: super::task::NativeTask) -> Self {
        self.registry.fakes.push(task);
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
    #[cfg(feature = "yaml")]
    #[cfg_attr(docsrs, doc(cfg(feature = "yaml")))]
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

    /// Serves a [`Listener`](super::Listener): HTTP routes or a gRPC service whose requests start workflows. The
    /// listener binds its port in [`build`](Self::build), which then has to run inside a tokio runtime.
    #[must_use]
    #[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "http-listener", feature = "grpc-listener")))
    )]
    pub fn listener(mut self, listener: super::listener::Listener) -> Self {
        self.listeners.push(listener);
        self
    }

    /// Starts a workflow on a schedule while the engine runs (see [`Schedule`](super::Schedule)). Schedules start when
    /// the engine is built inside a tokio runtime.
    #[must_use]
    pub fn schedule(mut self, schedule: super::schedule::Schedule) -> Self {
        self.schedules.push(schedule);
        self
    }

    /// Keeps finished instances for `period` after their last change; [`Engine::sweep`] purges older ones.
    #[must_use]
    pub fn retention(mut self, period: std::time::Duration) -> Self {
        self.retention = Some(period);
        self
    }

    /// Passes everything the engine stores (journals and history payloads) through `codec`, for example to encrypt
    /// it. An engine reading those records needs the same codec.
    #[must_use]
    pub fn codec(mut self, codec: impl PayloadCodec) -> Self {
        self.codec = Some(Arc::new(codec));
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
        let mut registry = self.registry;
        registry.add_builtins();
        registry.apply_fakes();
        #[cfg(feature = "yaml")]
        {
            registry.validate(&self.catalogs)?;
            self.catalogs.validate()?;
        }
        #[cfg(not(feature = "yaml"))]
        registry.validate()?;

        let persistence = self.persistence.unwrap_or_else(|| {
            Arc::new(InMemoryPersistence::new()) as Arc<dyn PersistenceProvider>
        });
        let cache = self
            .cache
            .unwrap_or_else(|| Arc::new(InMemoryCache::new()) as Arc<dyn CacheProvider>);

        let clock = self.clock.unwrap_or_else(|| Arc::new(SystemClock));
        // Chosen here, as the YAML interpreter's store checks its writes against the leases it names.
        let owner = self
            .engine_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // YAML workflows run on the YAML interpreter, which calls native tasks and custom executors by name.
        #[cfg(feature = "yaml")]
        let yaml = {
            // With leases, the interpreter writes for an instance only while this engine holds its lease.
            let interpreter_store: Arc<dyn PersistenceProvider> = match self.lease {
                Some(ttl) if persistence.supports_leases() => Arc::new(super::fence::LeaseFence {
                    inner: Arc::clone(&persistence),
                    owner: owner.clone(),
                    ttl,
                    clock: Arc::clone(&clock),
                    held: std::sync::Mutex::new(HashMap::new()),
                }),
                Some(_) | None => Arc::clone(&persistence),
            };
            let mut legacy = DurableEngine::new_with_config(
                interpreter_store,
                Arc::clone(&cache),
                self.event_buffer,
            )
            .map_err(|e| Error::Engine(e.to_string()))?;
            let mut natives: HashMap<String, Box<dyn Executor>> = registry
                .tasks
                .iter()
                .map(|(name, task)| {
                    (
                        (*name).to_string(),
                        Box::new(NativeExecutor(task.clone())) as Box<dyn Executor>,
                    )
                })
                .collect();
            for (name, executor) in &registry.executors {
                natives.insert(
                    name.clone(),
                    Box::new(CustomExecutor {
                        name: name.clone(),
                        executor: Arc::clone(executor),
                    }),
                );
            }
            legacy.add_executors(natives);
            legacy.add_workflows(self.yaml_workflows);
            legacy.set_clock(Arc::clone(&clock));
            YamlRuntime {
                catalogs: self.catalogs,
                legacy,
            }
        };
        #[cfg(not(feature = "yaml"))]
        let yaml = YamlRuntime {};

        let mut shared = Shared::new(
            persistence,
            cache,
            registry,
            yaml,
            self.self_replay,
            clock,
            self.codec,
        );
        shared.retention = self.retention;
        if let Some(ttl) = self.lease {
            if !shared.persistence.supports_leases() {
                return Err(Error::invalid(
                    "`lease` needs a store that supports leases (all built-in stores do)",
                ));
            }
            shared.leasing = Some(super::runtime::Leasing {
                owner,
                ttl,
                poll: self.poll,
            });
        }
        let shared = Arc::new(shared);
        super::runtime::register_on_store(&shared)?;
        #[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
        for listener in self.listeners {
            let serving = listener.start(Arc::downgrade(&shared))?;
            super::runtime::lock(&shared.listeners).push(Arc::new(serving));
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            for schedule in self.schedules {
                shared.spawn_background(super::schedule::drive(schedule, Arc::downgrade(&shared)));
            }
            if shared.leasing.is_some() {
                shared.spawn_background(super::runtime::tend_leases(Arc::downgrade(&shared)));
            }
        }
        Ok(Engine { shared })
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
        self.shared.check_accepting()?;
        let value = serde_json::to_value(&input)
            .map_err(|e| Error::invalid(format!("the input could not be serialized: {e}")))?;
        let launched = match runnable.target() {
            RunTarget::Native(info) => {
                let entry = self.shared.find_workflow(&info.id)?;
                let id = uuid::Uuid::new_v4().to_string();
                let journal = Journal::new(info.id.clone(), value.clone());
                self.shared.launch(id, entry, value, journal).await?
            }
            #[cfg(feature = "yaml")]
            RunTarget::Definition(definition) => {
                self.shared.launch_definition(&definition, value).await?
            }
            #[cfg(feature = "yaml")]
            RunTarget::Catalog { name, version } => {
                let definition = self.shared.yaml.catalogs.load(name, version).await?;
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
                "workflow instance `{id}` was not found in the store"
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
        if let Some((holder, _)) = self.shared.take_lease(id).await? {
            return Ok(self.wrap(Launched {
                core: self.remote(id, journal, holder),
            }));
        }
        let entry = self.shared.find_workflow(&journal.workflow)?;
        let input = journal.input.clone();
        let launched = self
            .shared
            .launch(id.to_string(), entry, input, journal)
            .await?;
        Ok(self.wrap(launched))
    }

    /// A handle's view of an instance another engine runs: signals and cancellation go through the store, and the
    /// result is read from the store once the instance ends.
    fn remote(&self, id: &str, journal: Journal, holder: String) -> Arc<InstanceCore> {
        let core = InstanceCore::new(
            id.to_string(),
            journal.workflow.clone(),
            super::workflow::ReplayMode::Strict,
            Arc::downgrade(&self.shared),
            journal,
            None,
            true,
        );
        let _ = core.remote.set(holder);
        let watched = Arc::clone(&core);
        let shared = Arc::downgrade(&self.shared);
        self.shared.spawn_background(async move {
            loop {
                let poll = {
                    let Some(shared) = shared.upgrade() else {
                        return;
                    };
                    shared
                        .leasing
                        .as_ref()
                        .map_or(std::time::Duration::from_secs(1), |l| l.poll)
                };
                tokio::time::sleep(poll).await;
                if watched.has_ended() {
                    return;
                }
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                if let Ok(Some(journal)) = shared.load_journal(&watched.id).await
                    && let Some(ended) = journal.ended()
                {
                    watched.log.close();
                    watched.publish(ended);
                    return;
                }
            }
        });
        core
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
        #[cfg_attr(not(feature = "yaml"), allow(clippy::infallible_destructuring_match))]
        let info = match runnable.target() {
            RunTarget::Native(info) => info,
            #[cfg(feature = "yaml")]
            RunTarget::Definition(definition) => {
                // Serialized before awaiting, so the future does not hold the input (which need not be `Sync`).
                let value = serde_json::to_value(&input).map_err(|e| {
                    Error::invalid(format!("the input could not be serialized: {e}"))
                })?;
                return self
                    .execute_definition_with(&definition, value, options)
                    .await;
            }
            #[cfg(feature = "yaml")]
            RunTarget::Catalog { .. } => {
                return Err(Error::invalid(
                    "execute_with starts Rust and YAML workflows; run catalog workflows with `execute`",
                ));
            }
        };
        let value = serde_json::to_value(&input)
            .map_err(|e| Error::invalid(format!("the input could not be serialized: {e}")))?;
        self.shared.check_accepting()?;
        let _starting = self.shared.starts.lock().await;
        let id = options
            .id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if let Some(journal) = self.shared.load_journal(&id).await? {
            if options.create_only {
                return Err(Error::invalid(format!("instance `{id}` already exists")));
            }
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
            // A new journal is written out in full by its first persist, deadline included.
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
            .ok_or_else(|| Error::Engine(format!("workflow instance `{id}` was not found")))?;
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
            if self.shared.take_lease(&summary.id).await?.is_some() {
                // Another engine holds the instance's lease: it is running there.
                continue;
            }
            match self.shared.find_workflow(&journal.workflow) {
                Ok(entry) => {
                    // The summary is only the journal's header; resuming needs its whole history.
                    let journal = self
                        .shared
                        .load_journal(&summary.id)
                        .await?
                        .unwrap_or(journal);
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
        #[cfg(feature = "yaml")]
        self.recover_yaml(&mut recovery).await?;
        Ok(recovery)
    }

    /// `execute_with` for a YAML workflow: an id of the caller's choosing. A YAML workflow sets its own deadline (its
    /// `timeout`), so a `timeout` option is rejected.
    #[cfg(feature = "yaml")]
    async fn execute_definition_with<O: DeserializeOwned + Send + 'static>(
        &self,
        definition: &crate::dsl::workflow::WorkflowDefinition,
        value: Value,
        options: ExecuteOptions,
    ) -> Result<ExecutionHandle<O>, Error> {
        if options.timeout.is_some() {
            return Err(Error::invalid(
                "a YAML workflow's deadline is its `timeout`; `ExecuteOptions::timeout` applies to Rust workflows",
            ));
        }
        self.shared.check_accepting()?;
        let _starting = self.shared.starts.lock().await;
        if let Some(id) = &options.id {
            if let Some(core) = self.shared.instance(id)
                && !options.create_only
            {
                return Ok(self.wrap(Launched { core }));
            }
            if self.shared.persistence.get_checkpoint(id).await?.is_some() {
                return Err(Error::invalid(format!("instance `{id}` already exists")));
            }
        }
        let launched = self
            .shared
            .launch_definition_as(definition, value, options.id)
            .await?;
        Ok(self.wrap(launched))
    }

    /// Resumes the YAML instances that were running. The YAML engine keeps its own record (events and a checkpoint),
    /// which names the workflow; its definition has to be registered with [`EngineBuilder::yaml_workflow`].
    #[cfg(feature = "yaml")]
    async fn recover_yaml(&self, recovery: &mut Recovery) -> Result<(), Error> {
        for id in self.yaml_ids().await? {
            if self.shared.instance(&id).is_some() {
                continue;
            }
            let Some(record) = self.yaml_record(&id).await? else {
                continue;
            };
            if record.info.status != Status::Running {
                continue;
            }
            if self.shared.take_lease(&id).await?.is_some() {
                // Another engine holds the instance's lease: it is running there.
                continue;
            }
            match self.shared.yaml.legacy.registered(&record.key).await {
                Some(definition) => {
                    self.shared
                        .resume_definition(&definition, id.clone(), record.input)
                        .await?;
                    recovery.resumed.push(id);
                }
                None => recovery.unrecoverable.push(Unrecoverable {
                    reason: format!(
                        "YAML workflow {} is not registered on this engine (EngineBuilder::yaml_workflow)",
                        record.key
                    ),
                    workflow: record.key,
                    id,
                }),
            }
        }
        Ok(())
    }

    /// The ids in the store that are not native journals: the YAML engine's instances.
    #[cfg(feature = "yaml")]
    async fn yaml_ids(&self) -> Result<Vec<String>, Error> {
        let native: std::collections::HashSet<String> = self
            .shared
            .summaries()
            .await?
            .into_iter()
            .map(|summary| summary.id)
            .collect();
        Ok(self
            .shared
            .indexed()
            .await?
            .into_iter()
            .filter(|id| !native.contains(id))
            .collect())
    }

    /// What the YAML engine's stored events say about instance `id`; `None` if it has no `WorkflowStarted`.
    #[cfg(feature = "yaml")]
    async fn yaml_record(&self, id: &str) -> Result<Option<YamlRecord>, Error> {
        use crate::workflow::WorkflowEvent;
        let events = self.shared.persistence.get_events(id).await?;
        let Some((key, input, started_at)) = events.iter().find_map(|event| {
            if let WorkflowEvent::WorkflowStarted {
                workflow_id,
                initial_data,
                timestamp,
                ..
            } = event
            {
                Some((workflow_id.clone(), initial_data.clone(), *timestamp))
            } else {
                None
            }
        }) else {
            return Ok(None);
        };
        let status = events
            .iter()
            .find_map(|event| {
                if matches!(event, WorkflowEvent::WorkflowCompleted { .. }) {
                    Some(Status::Completed)
                } else if matches!(event, WorkflowEvent::WorkflowFailed { .. }) {
                    Some(Status::Faulted)
                } else if matches!(event, WorkflowEvent::WorkflowCancelled { .. }) {
                    Some(Status::Cancelled)
                } else {
                    None
                }
            })
            .unwrap_or(Status::Running);
        // The key is `namespace/name/version`, as `WorkflowStarted` records it.
        let mut parts = key.splitn(3, '/');
        let workflow = WorkflowId {
            namespace: parts.next().unwrap_or_default().to_string(),
            name: parts.next().unwrap_or_default().to_string(),
            version: parts.next().unwrap_or_default().to_string(),
        };
        Ok(Some(YamlRecord {
            info: InstanceInfo {
                id: id.to_string(),
                workflow,
                status,
                started_at: Some(started_at),
                updated_at: events.iter().map(WorkflowEvent::timestamp).max(),
                parent: None,
            },
            key,
            input,
        }))
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
        #[cfg(feature = "yaml")]
        for id in self.yaml_ids().await? {
            if let Some(record) = self.yaml_record(&id).await?
                && filter.matches(&record.info)
            {
                found.push(record.info);
            }
        }
        found.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
        Ok(found
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit.unwrap_or(usize::MAX))
            .collect())
    }

    /// What the engine knows about one instance: its status, its run, how far it got, and what it is waiting for.
    ///
    /// # Errors
    /// If there is no such instance or the store cannot be read.
    pub async fn describe(&self, id: &str) -> Result<InstanceDetails, Error> {
        let Some(journal) = self.shared.load_journal(id).await? else {
            #[cfg(feature = "yaml")]
            if let Some(record) = self.yaml_record(id).await? {
                // The YAML engine keeps no journal: its steps are its completed tasks, and its timers are not listed.
                let steps = self
                    .shared
                    .persistence
                    .get_events(id)
                    .await?
                    .iter()
                    .filter(|event| {
                        matches!(event, crate::workflow::WorkflowEvent::TaskCompleted { .. })
                    })
                    .count();
                let status = match self.shared.instance(id) {
                    Some(core) => core.status().into(),
                    None => record.info.status,
                };
                return Ok(InstanceDetails {
                    id: record.info.id,
                    workflow: record.info.workflow,
                    status,
                    run: 1,
                    steps,
                    timers: Vec::new(),
                    pending_events: 0,
                    started_at: record.info.started_at,
                    updated_at: record.info.updated_at,
                    parent: None,
                });
            }
            return Err(Error::Engine(format!(
                "workflow instance `{id}` was not found"
            )));
        };
        Ok(InstanceDetails {
            id: id.to_string(),
            status: journal.status.into(),
            run: journal.run.max(1),
            steps: journal.steps.len(),
            timers: journal
                .deadlines
                .iter()
                .map(|(key, at)| PendingTimer {
                    key: key.clone(),
                    fires_at: *at,
                })
                .collect(),
            pending_events: journal
                .inbox
                .values()
                .map(std::collections::VecDeque::len)
                .sum(),
            started_at: journal.created_at,
            updated_at: journal.updated_at,
            parent: journal.parent.clone(),
            workflow: journal.workflow,
        })
    }

    pub(crate) fn from_shared(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Stops a schedule from firing until [`resume_schedule`](Self::resume_schedule). Stored, so it holds across
    /// restarts and for every engine on the store.
    ///
    /// # Errors
    /// If the store cannot be written.
    pub async fn pause_schedule(&self, name: &str) -> Result<(), Error> {
        super::schedule::update_state(self, name, |state| state.paused = true).await
    }

    /// Lets a paused schedule fire again. Fire times that passed while it was paused follow its catch-up policy.
    ///
    /// # Errors
    /// If the store cannot be written.
    pub async fn resume_schedule(&self, name: &str) -> Result<(), Error> {
        super::schedule::update_state(self, name, |state| state.paused = false).await
    }

    /// Removes a schedule for good: it does not fire again, here or after a restart.
    ///
    /// # Errors
    /// If the store cannot be written.
    pub async fn remove_schedule(&self, name: &str) -> Result<(), Error> {
        super::schedule::update_state(self, name, |state| state.removed = true).await
    }

    /// Removes a finished instance (its journal, history and checkpoint) from the store.
    ///
    /// # Errors
    /// If the instance is still running, does not exist, or the store cannot delete it.
    pub async fn purge(&self, id: &str) -> Result<(), Error> {
        let journal = self
            .shared
            .load_journal(id)
            .await?
            .ok_or_else(|| Error::Engine(format!("workflow instance `{id}` was not found")))?;
        if journal.status == JournalStatus::Running || self.shared.instance(id).is_some() {
            return Err(Error::invalid(format!(
                "instance `{id}` is still running; cancel it or let it finish before purging it"
            )));
        }
        self.shared.persistence.delete_instance(id).await?;
        Ok(())
    }

    /// Purges every finished instance (Rust or YAML) whose last change is older than the engine's
    /// [`retention`](EngineBuilder::retention) period, and returns their ids. Running instances are never purged. The
    /// events published on the engine, which a YAML `listen` task that resumes after a restart reads, are kept for the
    /// same period, by day.
    /// Call it periodically (for example from a timer in your service).
    ///
    /// # Errors
    /// If the store cannot be read or written.
    pub async fn sweep(&self) -> Result<Vec<String>, Error> {
        let Some(retention) = self.shared.retention else {
            return Ok(Vec::new());
        };
        let now = self.shared.clock.now();
        let mut purged = Vec::new();
        for summary in self.shared.summaries().await? {
            let journal = &summary.journal;
            let old_enough = journal
                .updated_at
                .is_some_and(|at| super::clock::after(at, retention) <= now);
            if journal.status != JournalStatus::Running && old_enough {
                self.shared.persistence.delete_instance(&summary.id).await?;
                purged.push(summary.id);
            }
        }
        #[cfg(feature = "yaml")]
        {
            for id in self.yaml_ids().await? {
                if self.shared.instance(&id).is_some() {
                    continue;
                }
                let Some(record) = self.yaml_record(&id).await? else {
                    continue;
                };
                let old_enough = record
                    .info
                    .updated_at
                    .is_some_and(|at| super::clock::after(at, retention) <= now);
                if record.info.status != Status::Running && old_enough {
                    self.shared.persistence.delete_instance(&id).await?;
                    purged.push(id);
                }
            }
            // A resumed `listen` task reads the events published since it started, so they are kept as long as
            // finished instances are.
            if let Some(before) = chrono::Duration::from_std(retention)
                .ok()
                .and_then(|retention| now.checked_sub_signed(retention))
            {
                self.shared
                    .yaml
                    .legacy
                    .prune_emitted(before)
                    .await
                    .map_err(|e| Error::Engine(e.to_string()))?;
            }
        }
        Ok(purged)
    }

    /// Stops the engine: new work is refused, task steps that are running get up to `grace` (on the engine's clock) to
    /// finish and record their result, and then every instance is stopped *without* ending it, so the next process
    /// over the same store resumes it with [`recover`](Self::recover). Sleeps and waits are not waited for.
    pub async fn shutdown(&self, grace: std::time::Duration) {
        // Listeners first: they stop accepting, and the requests they are serving finish their workflows.
        #[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
        {
            let listeners: Vec<_> = super::runtime::lock(&self.shared.listeners)
                .drain(..)
                .collect();
            for listener in listeners {
                listener.stop(grace).await;
            }
        }
        self.shared
            .shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let active = Arc::clone(&self.shared.active_tasks);
        let deadline = super::clock::after(self.shared.clock.now(), grace);
        let out_of_time = self.shared.clock.sleep_until(deadline);
        tokio::pin!(out_of_time);
        while active.count() > 0 {
            tokio::select! {
                () = active.changed() => {}
                () = &mut out_of_time => break,
            }
        }
        // Task steps count as in flight until their result is recorded, so nothing finished is lost here.
        self.shared.abandon_all();
    }

    /// The address listener `name` is serving on: with port `0`, the port the OS assigned. `None` if there is no such
    /// listener, or it was stopped by [`shutdown`](Self::shutdown).
    #[cfg(any(feature = "http-listener", feature = "grpc-listener"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "http-listener", feature = "grpc-listener")))
    )]
    #[must_use]
    pub fn listener_addr(&self, name: &str) -> Option<std::net::SocketAddr> {
        super::runtime::lock(&self.shared.listeners)
            .iter()
            .find(|listener| listener.name == name)
            .map(|listener| listener.addr)
    }

    /// Registers a YAML workflow that other YAML workflows run by name (`run: workflow:`), on a running engine. See
    /// [`EngineBuilder::yaml_workflow`].
    #[cfg(feature = "yaml")]
    #[cfg_attr(docsrs, doc(cfg(feature = "yaml")))]
    pub async fn register_yaml_workflow<I, O>(&self, workflow: &super::workflow::Workflow<I, O>) {
        // Registration only inserts into a map; the YAML engine reports no error for it.
        let _ = self
            .shared
            .yaml
            .legacy
            .register_workflow(workflow.definition().clone())
            .await;
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
        if let Some(journal) = self.shared.load_journal(id).await? {
            return Ok(journal.status.into());
        }
        #[cfg(feature = "yaml")]
        if let Some(record) = self.yaml_record(id).await? {
            return Ok(record.info.status);
        }
        Err(Error::Engine(format!(
            "workflow instance `{id}` was not found"
        )))
    }

    /// The events recorded for an instance.
    ///
    /// # Errors
    /// If the store cannot be read.
    pub async fn history(&self, id: &str) -> Result<Vec<Event>, Error> {
        let codec = self.shared.codec.as_deref();
        self.shared
            .persistence
            .get_events(id)
            .await?
            .into_iter()
            .map(|stored| {
                super::runtime::map_payloads(stored, |value| super::codec::decode(codec, value))
                    .map(Event::from_stored)
            })
            .collect()
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

    /// Fails for a handle to an instance another engine runs (see [`EngineBuilder::lease`]): `what` needs that engine.
    fn local(&self, what: &str) -> Result<(), Error> {
        match self.core.remote.get() {
            Some(owner) => Err(Error::invalid(format!(
                "instance `{}` runs on engine `{owner}`; {what} need the engine that runs it",
                self.core.id
            ))),
            None => Ok(()),
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
        if self.core.remote.get().is_some() {
            let message = serde_json::to_value(super::runtime::Message::Cancel)
                .map_err(|e| Error::Engine(e.to_string()))?;
            return Ok(self
                .shared
                .persistence
                .post_message(&self.core.id, message)
                .await?);
        }
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
        self.local("updates")?;
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
        self.local("queries")?;
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
        if self.core.remote.get().is_some() {
            let message = serde_json::to_value(super::runtime::Message::Event {
                name: name.to_string(),
                payload: value,
            })
            .map_err(|e| Error::Engine(e.to_string()))?;
            return Ok(self
                .shared
                .persistence
                .post_message(&self.core.id, message)
                .await?);
        }
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

/// A YAML instance as the YAML engine's stored events describe it.
#[cfg(feature = "yaml")]
struct YamlRecord {
    info: InstanceInfo,
    /// `namespace/name/version`, the key its definition is registered under.
    key: String,
    input: Value,
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
    /// Fail with `Error::Invalid` instead of returning an existing instance (used by schedules).
    pub(crate) create_only: bool,
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

    /// Fails instead of returning an existing instance with the same id.
    #[must_use]
    pub(crate) fn create_only(mut self) -> Self {
        self.create_only = true;
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

/// One instance in detail, from [`Engine::describe`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InstanceDetails {
    /// The instance id.
    pub id: String,
    /// The workflow it runs.
    pub workflow: WorkflowId,
    /// Where it is in its life.
    pub status: Status,
    /// Which run it is on (see `continue_as_new`); the first run is 1.
    pub run: u32,
    /// How many steps the current run has recorded.
    pub steps: usize,
    /// Durable timers and step timeouts that have not fired yet.
    pub timers: Vec<PendingTimer>,
    /// Events sent to the instance that no `wait_for_event` has consumed yet.
    pub pending_events: usize,
    /// When it started.
    pub started_at: Option<DateTime<Utc>>,
    /// When its journal last changed.
    pub updated_at: Option<DateTime<Utc>>,
    /// The instance that runs it as a child workflow.
    pub parent: Option<String>,
}

/// A durable deadline of an instance: a sleep (`sleep:<step>`), a step timeout (`timeout:<step>`) or the workflow's
/// own deadline (`workflow`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PendingTimer {
    /// What the deadline belongs to.
    pub key: String,
    /// When it fires.
    pub fires_at: DateTime<Utc>,
}

/// Does nothing. The YAML interpreter's task-by-task trace is now [`tracing`](https://docs.rs/tracing) events on the
/// `jackdaw::yaml` target (see the [events chapter](crate::guide::events) of the guide), which a subscriber shows or
/// filters like any other.
#[cfg(feature = "yaml")]
#[cfg_attr(docsrs, doc(cfg(feature = "yaml")))]
#[deprecated(
    since = "0.2.1",
    note = "the YAML trace is `tracing` events on the `jackdaw::yaml` target; enable that target in your subscriber"
)]
pub fn set_yaml_trace(_enabled: bool) {}
