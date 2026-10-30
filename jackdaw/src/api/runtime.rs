//! The runtime for Rust workflows: instances, the journal, steps, and replay.
//!
//! A workflow's `#[run]` is ordinary async Rust. To make it durable, every step it takes (a task call, a child
//! workflow, a sleep, a wait for an event, a clock reading) is recorded in a journal as it completes. When an instance
//! has to be rebuilt, `#[run]` runs again from the top: steps that have a record return it, and the first step without
//! one runs live. Step identities are assigned when the step is *created* (in program order), not when it is polled,
//! so parallel combinators such as `futures::try_join!` replay correctly.
use super::catalog::{CatalogDef, CatalogEntry, EntryKind};
use super::clock::SharedClock;
use super::context::WorkflowContext;
use super::error::{Divergence, Error, Fault, StepError, error_types};
use super::event::{Event, EventLog};
use super::task::{BoxFuture, NativeTask, Retry, TaskContext, TaskOptions, TaskTarget};
use super::workflow::{
    Dependency, ReplayMode, RunTarget, SyncWorkflowContext, WorkflowContextView, WorkflowDef,
    WorkflowId,
};
use crate::cache::{CacheEntry, CacheProvider, compute_cache_key};
use crate::durableengine::DurableEngine;
use crate::persistence::PersistenceProvider;
use crate::workflow::WorkflowCheckpoint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use sha2::{Digest, Sha256};
use std::any::TypeId;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::marker::PhantomData;
use std::panic::Location;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

/// Call types the engine provides itself; a native task may not take one of these names.
pub(crate) const BUILTIN_CALL_TYPES: &[&str] = &[
    "http",
    "rest",
    "openapi",
    "grpc",
    "asyncapi",
    "python",
    "javascript",
    "a2a",
    "mcp",
];

const INDEX_ID: &str = "__jackdaw_index__";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------------------------------
// Registry: what the engine knows how to run
// ---------------------------------------------------------------------------------------------------------------

pub(crate) trait Runner: Send + Sync {
    fn spawn(
        &self,
        core: Arc<InstanceCore>,
        input: Value,
    ) -> BoxFuture<'static, Result<Value, Error>>;
}

struct TypedRunner<W>(PhantomData<fn() -> W>);

impl<W: WorkflowDef> Runner for TypedRunner<W> {
    fn spawn(
        &self,
        core: Arc<InstanceCore>,
        input: Value,
    ) -> BoxFuture<'static, Result<Value, Error>> {
        Box::pin(drive::<W>(core, input))
    }
}

pub(crate) struct WorkflowEntry {
    pub id: WorkflowId,
    type_id: TypeId,
    pub replay: ReplayMode,
    explicit: bool,
    pub dependencies: Vec<Dependency>,
    pub runner: Arc<dyn Runner>,
}

/// The tasks and workflows registered on an engine. Public only so a workflow can name how to register itself.
#[derive(Default)]
pub struct Registry {
    pub(crate) tasks: HashMap<&'static str, NativeTask>,
    pub(crate) workflows: HashMap<String, WorkflowEntry>,
    pub(crate) errors: Vec<String>,
}

impl Registry {
    pub(crate) fn add_task(&mut self, task: NativeTask) {
        let name = task.name;
        if BUILTIN_CALL_TYPES.contains(&name) {
            self.errors.push(format!(
                "a task cannot be named `{name}`: that is a built-in call type"
            ));
        } else if self.tasks.insert(name, task).is_some() {
            self.errors
                .push(format!("task `{name}` is registered twice"));
        }
    }

    pub(crate) fn add_workflow<W: WorkflowDef>(&mut self, explicit: bool) {
        let id = WorkflowId::of::<W>();
        let key = id.key();
        if let Some(existing) = self.workflows.get_mut(&key) {
            if existing.type_id != TypeId::of::<W>() {
                self.errors
                    .push(format!("two different workflows have the identity {id}"));
            } else if explicit && existing.explicit {
                self.errors
                    .push(format!("workflow {id} is registered twice"));
            } else if explicit {
                existing.explicit = true;
            }
            return;
        }
        let dependencies = W::dependencies();
        let children: Vec<fn(&mut Registry)> = dependencies
            .iter()
            .filter_map(|dependency| match dependency {
                Dependency::Workflow(info) => Some(info.register),
                Dependency::Task(_) | Dependency::CatalogWorkflow { .. } => None,
            })
            .collect();
        self.workflows.insert(
            key,
            WorkflowEntry {
                id,
                type_id: TypeId::of::<W>(),
                replay: W::REPLAY,
                explicit,
                dependencies,
                runner: Arc::new(TypedRunner::<W>(PhantomData)),
            },
        );
        for register in children {
            register(self);
        }
    }

    /// Checks that everything a registered workflow refers to exists.
    pub(crate) fn validate(&self, catalogs: &CatalogSet) -> Result<(), Error> {
        let mut problems = self.errors.clone();
        for entry in self.workflows.values() {
            for dependency in &entry.dependencies {
                match dependency {
                    Dependency::Task(TaskTarget::Native(name))
                        if !self.tasks.contains_key(name) =>
                    {
                        problems.push(format!(
                            "workflow {} calls task `{name}`, which is not registered",
                            entry.id
                        ))
                    }
                    Dependency::Task(TaskTarget::Catalog { name, version })
                    | Dependency::CatalogWorkflow { name, version }
                        if !catalogs.contains(name, version) =>
                    {
                        problems.push(format!(
                            "workflow {} uses catalog entry `{name}:{version}`, which no registered catalog declares",
                            entry.id
                        ));
                    }
                    Dependency::Task(_)
                    | Dependency::Workflow(_)
                    | Dependency::CatalogWorkflow { .. } => {}
                }
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(Error::invalid(problems.join("; ")))
        }
    }
}

/// Registers workflow `W` (used for child workflows found by the macro).
pub fn register_workflow<W: WorkflowDef>(registry: &mut Registry) {
    registry.add_workflow::<W>(false);
}

// ---------------------------------------------------------------------------------------------------------------
// Catalogs
// ---------------------------------------------------------------------------------------------------------------

struct BoundEntry {
    source: &'static str,
    entry: CatalogEntry,
}

#[derive(Default)]
pub(crate) struct CatalogSet {
    entries: HashMap<(String, String), BoundEntry>,
}

impl CatalogSet {
    pub(crate) fn add<C: CatalogDef>(&mut self) {
        for entry in C::entries() {
            self.entries.insert(
                (entry.name.to_string(), entry.version.to_string()),
                BoundEntry {
                    source: C::SOURCE,
                    entry,
                },
            );
        }
    }

    fn contains(&self, name: &str, version: &str) -> bool {
        self.entries
            .contains_key(&(name.to_string(), version.to_string()))
    }

    fn file_of(source: &str, name: &str, version: &str) -> Option<String> {
        source.strip_prefix("file://").map(|path| {
            format!(
                "{}/{name}/{version}/function.yaml",
                path.trim_end_matches('/')
            )
        })
    }

    /// Every declared entry must exist, and must not contradict the Rust types it is bound to.
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let mut problems = Vec::new();
        for bound in self.entries.values() {
            let (name, version) = (bound.entry.name, bound.entry.version);
            let Some(file) = Self::file_of(bound.source, name, version) else {
                continue; // remote catalogs are checked when first used
            };
            let text = match std::fs::read_to_string(&file) {
                Ok(text) => text,
                Err(_) => {
                    problems.push(format!(
                        "catalog entry `{name}:{version}` was not found at {file}"
                    ));
                    continue;
                }
            };
            let Ok(document) = serde_yaml::from_str::<Value>(&text) else {
                problems.push(format!(
                    "catalog entry `{name}:{version}` at {file} is not valid YAML"
                ));
                continue;
            };
            let kind = match bound.entry.kind {
                EntryKind::Task => "task",
                EntryKind::Workflow => "workflow",
            };
            if let Some(problem) =
                schema_conflict(name, version, kind, &document, &bound.entry.input_schema)
            {
                problems.push(problem);
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(Error::invalid(problems.join("; ")))
        }
    }

    pub(crate) async fn load(
        &self,
        name: &str,
        version: &str,
    ) -> Result<WorkflowDefinition, Error> {
        let Some(bound) = self.entries.get(&(name.to_string(), version.to_string())) else {
            return Err(Error::invalid(format!(
                "no registered catalog declares `{name}:{version}`"
            )));
        };
        let text = if let Some(file) = Self::file_of(bound.source, name, version) {
            tokio::fs::read_to_string(&file).await.map_err(|e| {
                Error::invalid(format!(
                    "cannot read catalog entry `{name}:{version}` at {file}: {e}"
                ))
            })?
        } else {
            let url = format!(
                "{}/functions/{name}/{version}/function.yaml",
                bound.source.trim_end_matches('/')
            );
            let response = reqwest::get(&url).await.map_err(|e| {
                Error::Engine(format!(
                    "cannot fetch catalog entry `{name}:{version}` from {url}: {e}"
                ))
            })?;
            response.text().await.map_err(|e| {
                Error::Engine(format!(
                    "cannot read catalog entry `{name}:{version}` from {url}: {e}"
                ))
            })?
        };
        serde_yaml::from_str(&text).map_err(|e| {
            Error::invalid(format!(
                "catalog entry `{name}:{version}` is not a valid workflow: {e}"
            ))
        })
    }
}

/// If the entry's input schema requires a property the Rust input type does not have, says so.
fn schema_conflict(
    name: &str,
    version: &str,
    kind: &str,
    document: &Value,
    rust_input: &Value,
) -> Option<String> {
    let required = document.pointer("/input/schema/required")?.as_array()?;
    let properties = rust_input.get("properties")?.as_object()?;
    required
        .iter()
        .filter_map(Value::as_str)
        .find(|field| !properties.contains_key(*field))
        .map(|field| {
            format!("catalog {kind} `{name}:{version}` requires input `{field}`, which the Rust input type it is bound to does not have")
        })
}

// ---------------------------------------------------------------------------------------------------------------
// The journal
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum JournalStatus {
    Running,
    Completed,
    Faulted,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Outcome {
    Ok(Value),
    Fault(Fault),
    Timeout(String),
}

impl Outcome {
    fn into_result(self) -> Result<Value, StepError> {
        match self {
            Self::Ok(value) => Ok(value),
            Self::Fault(fault) => Err(StepError::Fault(fault)),
            Self::Timeout(detail) => Err(StepError::Timeout { detail }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StepRecord {
    pub kind: String,
    pub name: String,
    pub ordinal: usize,
    pub input_hash: String,
    pub call_site: String,
    pub outcome: Outcome,
    /// Position in the instance's history: every record and every signal takes the next number. Replay uses it to
    /// apply signals at the point where they originally arrived.
    #[serde(default)]
    pub order: u64,
}

/// A signal as it was delivered, with its position in the history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SignalRecord {
    pub name: String,
    pub argument: Value,
    /// 0 for journals written before signals were ordered: those are applied before `#[run]` starts.
    #[serde(default)]
    pub order: u64,
}

/// How an instance ended, when it did not complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Failure {
    Fault(Fault),
    Timeout(String),
    Cancelled,
    Engine(String),
}

impl Failure {
    pub(crate) fn of(error: &Error) -> Self {
        match error {
            Error::Faulted(fault) => Self::Fault(fault.clone()),
            Error::Timeout { detail } => Self::Timeout(detail.clone()),
            Error::Cancelled => Self::Cancelled,
            other @ (Error::Invalid(_)
            | Error::NonDeterminism(_)
            | Error::Persistence(_)
            | Error::Cache(_)
            | Error::Engine(_)) => Self::Engine(other.to_string()),
        }
    }

    pub(crate) fn to_error(&self) -> Error {
        match self {
            Self::Fault(fault) => Error::Faulted(fault.clone()),
            Self::Timeout(detail) => Error::Timeout {
                detail: detail.clone(),
            },
            Self::Cancelled => Error::Cancelled,
            Self::Engine(message) => Error::Engine(message.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Journal {
    pub workflow: WorkflowId,
    pub input: Value,
    pub status: JournalStatus,
    pub steps: BTreeMap<usize, StepRecord>,
    pub signals: Vec<SignalRecord>,
    pub patches: BTreeMap<String, bool>,
    pub result: Option<Value>,
    /// Why the instance did not complete.
    #[serde(default)]
    pub failure: Option<Failure>,
    /// The instance that runs this one as a child step.
    #[serde(default)]
    pub parent: Option<String>,
    /// Absolute deadlines of durable timers and step timeouts that are in progress, by step key.
    #[serde(default)]
    pub deadlines: BTreeMap<String, DateTime<Utc>>,
    /// Events that were sent to the instance and not yet consumed by a `wait_for_event`.
    #[serde(default)]
    pub inbox: BTreeMap<String, VecDeque<Value>>,
    /// The last history position handed out (see [`StepRecord::order`]).
    #[serde(default)]
    pub history: u64,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

impl Journal {
    pub(crate) fn new(workflow: WorkflowId, input: Value) -> Self {
        Self {
            workflow,
            input,
            status: JournalStatus::Running,
            steps: BTreeMap::new(),
            signals: Vec::new(),
            patches: BTreeMap::new(),
            result: None,
            failure: None,
            parent: None,
            deadlines: BTreeMap::new(),
            inbox: BTreeMap::new(),
            history: 0,
            created_at: None,
            updated_at: None,
        }
    }

    fn next_order(&mut self) -> u64 {
        self.history += 1;
        self.history
    }

    /// Records a step. Strict replay finds records by position, so the position is the key. Lenient replay finds them
    /// by kind, name and ordinal, so a new record takes a fresh key instead of overwriting an older step's record.
    fn insert(&mut self, mode: ReplayMode, seq: usize, mut record: StepRecord) {
        record.order = self.next_order();
        let key = match mode {
            ReplayMode::Strict => seq,
            ReplayMode::Lenient => self.steps.keys().next_back().map_or(0, |last| last + 1),
        };
        self.steps.insert(key, record);
    }

    fn from_checkpoint(checkpoint: &WorkflowCheckpoint) -> Option<Self> {
        serde_json::from_value(checkpoint.data.get("jackdaw_native")?.clone()).ok()
    }

    /// The instance's outcome, if it has ended.
    pub(crate) fn ended(&self) -> Option<Result<Value, Error>> {
        match self.status {
            JournalStatus::Running => None,
            JournalStatus::Completed => Some(Ok(self.result.clone().unwrap_or(Value::Null))),
            JournalStatus::Cancelled => Some(Err(Error::Cancelled)),
            JournalStatus::Faulted => Some(Err(self.failure.as_ref().map_or_else(
                || Error::Engine("the workflow failed".to_string()),
                Failure::to_error,
            ))),
        }
    }
}

fn hash_of(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn site(location: &Location<'_>) -> String {
    format!("{}:{}", location.file(), location.line())
}

/// A copy of an instance's result for one more waiter. `Error` is not `Clone` because two variants wrap store errors;
/// those are passed on as their message.
pub(crate) fn duplicate(result: &Result<Value, Error>) -> Result<Value, Error> {
    match result {
        Ok(value) => Ok(value.clone()),
        Err(error) => Err(match error {
            Error::Faulted(fault) => Error::Faulted(fault.clone()),
            Error::Cancelled => Error::Cancelled,
            Error::Timeout { detail } => Error::Timeout {
                detail: detail.clone(),
            },
            Error::Invalid(message) => Error::Invalid(message.clone()),
            Error::NonDeterminism(divergence) => Error::NonDeterminism(divergence.clone()),
            other @ (Error::Persistence(_) | Error::Cache(_) | Error::Engine(_)) => {
                Error::Engine(other.to_string())
            }
        }),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Shared engine state
// ---------------------------------------------------------------------------------------------------------------

pub(crate) struct Shared {
    pub persistence: Arc<dyn PersistenceProvider>,
    pub cache: Arc<dyn CacheProvider>,
    pub registry: Registry,
    pub catalogs: CatalogSet,
    pub legacy: DurableEngine,
    pub self_replay: bool,
    pub clock: SharedClock,
    instances: Mutex<HashMap<String, Arc<InstanceCore>>>,
    running: Mutex<Vec<JoinHandle<()>>>,
    index: tokio::sync::Mutex<()>,
    /// Serializes starts by id, so two concurrent `execute_with_id` calls create one instance.
    pub starts: tokio::sync::Mutex<()>,
}

impl Drop for Shared {
    /// Dropping the engine abandons its instances: they are stopped without a terminal record, so a new engine over
    /// the same store can resume them.
    fn drop(&mut self) {
        for handle in lock(&self.running).drain(..) {
            handle.abort();
        }
    }
}

/// An instance that has been started.
pub(crate) struct Launched {
    pub core: Arc<InstanceCore>,
}

/// One instance found by [`Shared::summaries`].
#[derive(Debug, Clone)]
pub(crate) struct Summary {
    pub id: String,
    pub journal: Journal,
}

impl Shared {
    pub(crate) fn new(
        persistence: Arc<dyn PersistenceProvider>,
        cache: Arc<dyn CacheProvider>,
        registry: Registry,
        catalogs: CatalogSet,
        legacy: DurableEngine,
        self_replay: bool,
        clock: SharedClock,
    ) -> Self {
        Self {
            persistence,
            cache,
            registry,
            catalogs,
            legacy,
            self_replay,
            clock,
            instances: Mutex::new(HashMap::new()),
            running: Mutex::new(Vec::new()),
            index: tokio::sync::Mutex::new(()),
            starts: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn find_workflow(&self, id: &WorkflowId) -> Result<&WorkflowEntry, Error> {
        self.registry.workflows.get(&id.key()).ok_or_else(|| {
            Error::invalid(format!(
                "workflow {id} is not registered; add it with `EngineBuilder::workflow`"
            ))
        })
    }

    /// The instance if it is running in this engine.
    pub(crate) fn instance(&self, id: &str) -> Option<Arc<InstanceCore>> {
        lock(&self.instances).get(id).cloned()
    }

    fn remember(&self, core: &Arc<InstanceCore>, handle: JoinHandle<()>) {
        lock(&self.instances).insert(core.id.clone(), Arc::clone(core));
        let mut running = lock(&self.running);
        running.retain(|handle| !handle.is_finished());
        running.push(handle);
    }

    fn forget(&self, id: &str) {
        lock(&self.instances).remove(id);
    }

    /// Cancels an instance that runs in this engine, and its children.
    pub(crate) fn cancel(&self, id: &str) {
        let targets: Vec<Arc<InstanceCore>> = lock(&self.instances).values().cloned().collect();
        let mut doomed = vec![id.to_string()];
        let mut i = 0;
        while let Some(parent) = doomed.get(i).cloned() {
            for core in &targets {
                if lock(&core.journal).parent.as_deref() == Some(parent.as_str())
                    && !doomed.contains(&core.id)
                {
                    doomed.push(core.id.clone());
                }
            }
            i += 1;
        }
        for core in targets {
            if doomed.contains(&core.id) {
                core.request_cancel();
            }
        }
    }

    async fn add_to_index(&self, id: &str) -> Result<(), Error> {
        let _guard = self.index.lock().await;
        let mut instances = self.indexed().await?;
        if !instances.iter().any(|known| known == id) {
            instances.push(id.to_string());
            let checkpoint = WorkflowCheckpoint {
                instance_id: INDEX_ID.to_string(),
                current_task: String::new(),
                data: json!({ "instances": instances }),
                timestamp: Utc::now(),
            };
            self.persistence.save_checkpoint(checkpoint).await?;
        }
        Ok(())
    }

    pub(crate) async fn indexed(&self) -> Result<Vec<String>, Error> {
        Ok(match self.persistence.get_checkpoint(INDEX_ID).await? {
            Some(checkpoint) => checkpoint
                .data
                .get("instances")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        })
    }

    pub(crate) async fn load_journal(&self, id: &str) -> Result<Option<Journal>, Error> {
        if let Some(core) = self.instance(id) {
            return Ok(Some(lock(&core.journal).clone()));
        }
        Ok(self
            .persistence
            .get_checkpoint(id)
            .await?
            .as_ref()
            .and_then(Journal::from_checkpoint))
    }

    /// Every stored native instance with its journal.
    pub(crate) async fn summaries(&self) -> Result<Vec<Summary>, Error> {
        let mut found = Vec::new();
        for id in self.indexed().await? {
            if let Some(journal) = self.load_journal(&id).await? {
                found.push(Summary { id, journal });
            }
        }
        Ok(found)
    }

    /// Starts (or, with an existing journal, resumes) a native workflow instance. If the instance is already running
    /// in this engine, returns it instead of starting it twice.
    pub(crate) async fn launch(
        self: &Arc<Self>,
        id: String,
        entry: &WorkflowEntry,
        input: Value,
        mut journal: Journal,
    ) -> Result<Launched, Error> {
        if let Some(core) = self.instance(&id) {
            return Ok(Launched { core });
        }
        let now = self.clock.now();
        journal.created_at.get_or_insert(now);
        journal.updated_at = Some(now);
        let core = InstanceCore::new(
            id,
            entry.id.clone(),
            entry.replay,
            Arc::downgrade(self),
            journal,
            None,
            false,
        );
        core.persist().await?;
        self.add_to_index(&core.id).await?;

        let run = entry.runner.spawn(Arc::clone(&core), input);
        let runner = Arc::clone(&entry.runner);
        let weak = Arc::downgrade(self);
        let for_task = Arc::clone(&core);
        let handle = tokio::spawn(async move {
            let publish = PublishOnDrop(Arc::clone(&for_task));
            let mut result = run.await;
            if let (Ok(value), Some(shared)) = (&result, weak.upgrade())
                && shared.self_replay
                && let Err(error) = verify_replay(&for_task, runner, value.clone()).await
            {
                result = Err(error);
            }
            publish.0.publish(result);
            if let Some(shared) = weak.upgrade() {
                shared.forget(&publish.0.id);
            }
        });
        self.remember(&core, handle);
        Ok(Launched { core })
    }

    /// Runs a YAML definition on the YAML engine and mirrors its events and result into one instance.
    pub(crate) async fn launch_definition(
        self: &Arc<Self>,
        definition: &WorkflowDefinition,
        input: Value,
    ) -> Result<Launched, Error> {
        self.validate_definition(definition)?;
        let mut legacy = self
            .legacy
            .execute(definition.clone(), input.clone())
            .await
            .map_err(|e| Error::Engine(e.to_string()))?;
        let id = legacy.instance_id().to_string();
        let workflow = WorkflowId {
            namespace: definition.document.namespace.clone(),
            name: definition.document.name.clone(),
            version: definition.document.version.clone(),
        };
        let core = InstanceCore::new(
            id,
            workflow.clone(),
            ReplayMode::Strict,
            Arc::downgrade(self),
            Journal::new(workflow, input),
            None,
            true,
        );
        let for_task = Arc::clone(&core);
        let weak = Arc::downgrade(self);
        let handle = tokio::spawn(async move {
            let publish = PublishOnDrop(Arc::clone(&for_task));
            let mut result = Err(Error::Engine(
                "the workflow ended without producing a result".to_string(),
            ));
            while let Some(stored) = legacy.next_event().await {
                if let crate::workflow::WorkflowEvent::WorkflowCompleted { final_data, .. } =
                    &stored
                {
                    result = Ok(final_data.clone());
                } else if let crate::workflow::WorkflowEvent::WorkflowFailed { error, .. } = &stored
                {
                    result = Err(Error::Faulted(Fault::runtime(error)));
                } else if matches!(
                    stored,
                    crate::workflow::WorkflowEvent::WorkflowCancelled { .. }
                ) {
                    result = Err(Error::Cancelled);
                }
                let event = Event::from_stored(stored);
                let terminal = event.is_terminal();
                for_task.log.push(event);
                if terminal {
                    break;
                }
            }
            for_task.finish(&result);
            for_task.log.close();
            publish.0.publish(result);
            if let Some(shared) = weak.upgrade() {
                shared.forget(&publish.0.id);
            }
        });
        self.remember(&core, handle);
        Ok(Launched { core })
    }

    /// Runs a YAML definition to completion and returns its output.
    pub(crate) async fn run_definition(
        self: &Arc<Self>,
        definition: &WorkflowDefinition,
        input: Value,
    ) -> Result<Value, Error> {
        let launched = self.launch_definition(definition, input).await?;
        launched.core.outcome().await
    }

    /// Runs a native child workflow, resuming it if an earlier attempt left a journal.
    async fn run_native_child(
        self: &Arc<Self>,
        parent: &str,
        id: String,
        target: &WorkflowId,
        input: Value,
    ) -> Result<Value, Error> {
        let entry = self.find_workflow(target)?;
        let journal = match self.load_journal(&id).await? {
            Some(journal) if journal.workflow == *target => {
                if let Some(ended) = journal.ended() {
                    return ended;
                }
                journal
            }
            Some(_) | None => {
                let mut journal = Journal::new(target.clone(), input.clone());
                journal.parent = Some(parent.to_string());
                journal
            }
        };
        let launched = self.launch(id, entry, input, journal).await?;
        launched.core.outcome().await
    }

    /// Fails fast on a YAML workflow that calls something nobody provides.
    pub(crate) fn validate_definition(&self, definition: &WorkflowDefinition) -> Result<(), Error> {
        let mut unknown = Vec::new();
        let functions: HashSet<String> = definition
            .use_
            .as_ref()
            .and_then(|use_| use_.functions.as_ref())
            .map(|functions| functions.keys().cloned().collect())
            .unwrap_or_default();
        for call in crate::durableengine::called_names(definition) {
            let known = self.registry.tasks.contains_key(call.as_str())
                || BUILTIN_CALL_TYPES.contains(&call.as_str())
                || functions.contains(&call)
                || call.contains(':')
                || call.starts_with("http://")
                || call.starts_with("https://")
                || call.starts_with("file://");
            if !known {
                unknown.push(call);
            }
        }
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(Error::invalid(format!(
                "workflow {}/{} calls `{}`, which is not a registered task, a built-in call type, or one of its own functions",
                definition.document.namespace,
                definition.document.name,
                unknown.join("`, `")
            )))
        }
    }
}

/// Publishes "the engine stopped" if the task running an instance is aborted before it publishes a result, so nobody
/// waits forever on an instance whose engine was dropped.
struct PublishOnDrop(Arc<InstanceCore>);

impl Drop for PublishOnDrop {
    fn drop(&mut self) {
        if self.0.result.borrow().is_none() {
            self.0.publish(Err(Error::Engine(
                "the engine stopped before the workflow finished".to_string(),
            )));
        }
    }
}

/// Replays a finished instance against its own journal and checks that it reaches the same result.
async fn verify_replay(
    original: &Arc<InstanceCore>,
    runner: Arc<dyn Runner>,
    expected: Value,
) -> Result<(), Error> {
    let journal = lock(&original.journal).clone();
    let input = journal.input.clone();
    let core = InstanceCore::new(
        format!("{}#replay", original.id),
        original.workflow.clone(),
        original.replay,
        Weak::clone(&original.shared),
        journal,
        Some(ReplayOnly {
            allow_frontier: false,
        }),
        true,
    );
    let replayed = runner.spawn(core, input).await?;
    if replayed == expected {
        Ok(())
    } else {
        Err(Error::NonDeterminism(Box::new(Divergence {
            instance_id: original.id.clone(),
            workflow: original.workflow.to_string(),
            step: original.next_seq.load(Ordering::SeqCst),
            expected: format!("result {replayed}"),
            recorded: Some(format!("result {expected}")),
            call_site: "the end of #[run]".to_string(),
        })))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// One running workflow instance
// ---------------------------------------------------------------------------------------------------------------

pub(crate) struct ReplayOnly {
    /// Reaching a step with no record is the end of the history (true), or a divergence (false).
    pub allow_frontier: bool,
}

pub(crate) struct Dispatch {
    pub signal: Box<dyn Fn(&str, Value) -> Result<(), Error> + Send + Sync>,
    pub query: Box<dyn Fn(&str) -> Result<Value, Error> + Send + Sync>,
}

pub(crate) struct InstanceCore {
    pub id: String,
    pub workflow: WorkflowId,
    pub replay: ReplayMode,
    pub shared: Weak<Shared>,
    pub journal: Mutex<Journal>,
    pub log: Arc<EventLog>,
    pub ephemeral: bool,
    pub replay_only: Option<ReplayOnly>,
    pub clock: SharedClock,
    /// How many steps were already recorded when this run started.
    pub boundary: usize,
    next_seq: AtomicUsize,
    ordinals: Mutex<HashMap<(String, String), usize>>,
    pub cancel: watch::Sender<bool>,
    pub state_changed: Notify,
    /// Recorded signals that this incarnation has not applied yet (it is replaying the history before them).
    pending_signals: Mutex<VecDeque<SignalRecord>>,
    /// How many inbox entries per event name live `wait_for_event` steps have claimed but not yet recorded.
    claims: Mutex<HashMap<String, usize>>,
    inbox_changed: Notify,
    dispatch: OnceLock<Dispatch>,
    dispatch_ready: Notify,
    pub frontier: Notify,
    persist_lock: tokio::sync::Mutex<()>,
    result: watch::Sender<Option<Arc<Result<Value, Error>>>>,
}

/// A step, ready to run.
pub(crate) struct StepSpec {
    pub seq: usize,
    pub ordinal: usize,
    pub kind: &'static str,
    pub name: String,
    pub input: Value,
    pub location: &'static Location<'static>,
    pub exec: StepExec,
    /// Fails the step with a timeout when it runs longer than this. The deadline is durable.
    pub timeout: Option<Duration>,
}

pub(crate) enum StepExec {
    Task {
        target: TaskTarget,
        options: TaskOptions,
    },
    Child(RunTarget),
    Sleep(Duration),
    Event,
}

impl InstanceCore {
    pub(crate) fn new(
        id: String,
        workflow: WorkflowId,
        replay: ReplayMode,
        shared: Weak<Shared>,
        journal: Journal,
        replay_only: Option<ReplayOnly>,
        ephemeral: bool,
    ) -> Arc<Self> {
        let (cancel, _) = watch::channel(false);
        let (result, _) = watch::channel(None);
        let boundary = journal.steps.len();
        let clock = shared.upgrade().map_or_else(
            || Arc::new(super::clock::SystemClock) as SharedClock,
            |shared| Arc::clone(&shared.clock),
        );
        let pending = journal
            .signals
            .iter()
            .filter(|signal| signal.order > 0)
            .cloned()
            .collect();
        Arc::new(Self {
            id,
            workflow,
            replay,
            shared,
            journal: Mutex::new(journal),
            log: Arc::new(EventLog::default()),
            ephemeral,
            replay_only,
            clock,
            boundary,
            next_seq: AtomicUsize::new(0),
            ordinals: Mutex::new(HashMap::new()),
            cancel,
            state_changed: Notify::new(),
            pending_signals: Mutex::new(pending),
            claims: Mutex::new(HashMap::new()),
            inbox_changed: Notify::new(),
            dispatch: OnceLock::new(),
            dispatch_ready: Notify::new(),
            frontier: Notify::new(),
            persist_lock: tokio::sync::Mutex::new(()),
            result,
        })
    }

    pub(crate) fn shared(&self) -> Result<Arc<Shared>, Error> {
        self.shared
            .upgrade()
            .ok_or_else(|| Error::Engine("the engine was dropped".to_string()))
    }

    pub(crate) fn status(&self) -> JournalStatus {
        lock(&self.journal).status
    }

    /// Records how the instance ended. The event stream is closed separately, after the terminal event is in it.
    pub(crate) fn finish(&self, result: &Result<Value, Error>) {
        let mut journal = lock(&self.journal);
        match result {
            Ok(output) => {
                journal.status = JournalStatus::Completed;
                journal.result = Some(output.clone());
            }
            Err(Error::Cancelled) => {
                journal.status = JournalStatus::Cancelled;
                journal.failure = Some(Failure::Cancelled);
            }
            // A divergence leaves the journal resumable (once the code is fixed).
            Err(Error::NonDeterminism(_)) => journal.status = JournalStatus::Running,
            Err(error) => {
                journal.status = JournalStatus::Faulted;
                journal.failure = Some(Failure::of(error));
            }
        }
    }

    /// Makes the instance's result available to everyone waiting on it.
    pub(crate) fn publish(&self, result: Result<Value, Error>) {
        self.result.send_replace(Some(Arc::new(result)));
    }

    /// Whether a result has been published.
    pub(crate) fn has_ended(&self) -> bool {
        self.result.borrow().is_some()
    }

    /// Waits for the instance's result.
    pub(crate) async fn outcome(&self) -> Result<Value, Error> {
        let mut receiver = self.result.subscribe();
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return duplicate(&result);
            }
            if receiver.changed().await.is_err() {
                return Err(Error::Engine(
                    "the engine stopped before the workflow finished".to_string(),
                ));
            }
        }
    }

    // ---- creating steps (synchronous: this is what fixes a step's identity) ----

    pub(crate) fn spec(
        &self,
        kind: &'static str,
        name: String,
        input: Value,
        location: &'static Location<'static>,
        exec: StepExec,
    ) -> StepSpec {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let ordinal = {
            let mut ordinals = lock(&self.ordinals);
            let count = ordinals
                .entry((kind.to_string(), name.clone()))
                .or_insert(0);
            let ordinal = *count;
            *count += 1;
            ordinal
        };
        StepSpec {
            seq,
            ordinal,
            kind,
            name,
            input,
            location,
            exec,
            timeout: None,
        }
    }

    /// The key under which a step's durable deadlines are stored.
    fn step_key(&self, spec: &StepSpec) -> String {
        match self.replay {
            ReplayMode::Strict => spec.seq.to_string(),
            ReplayMode::Lenient => format!("{}/{}/{}", spec.kind, spec.name, spec.ordinal),
        }
    }

    fn find_record(&self, kind: &str, spec: &StepSpec) -> Option<StepRecord> {
        let journal = lock(&self.journal);
        match self.replay {
            ReplayMode::Strict => journal.steps.get(&spec.seq).cloned(),
            ReplayMode::Lenient => journal
                .steps
                .values()
                .find(|record| record.kind == kind && record.ordinal == spec.ordinal)
                .cloned(),
        }
    }

    /// A value that must be the same on replay (the clock, randomness, an id).
    pub(crate) fn recorded_value(
        &self,
        kind: &'static str,
        location: &'static Location<'static>,
        produce: impl FnOnce() -> Value,
    ) -> Value {
        let spec = self.spec(
            kind,
            kind.to_string(),
            Value::Null,
            location,
            StepExec::Event,
        );
        if let Some(record) = self.find_record(kind, &spec)
            && let Outcome::Ok(value) = &record.outcome
        {
            self.apply_signals_before(record.order);
            return value.clone();
        }
        self.apply_all_pending();
        let value = produce();
        lock(&self.journal).insert(
            self.replay,
            spec.seq,
            StepRecord {
                kind: kind.to_string(),
                name: kind.to_string(),
                ordinal: spec.ordinal,
                input_hash: String::new(),
                call_site: site(location),
                outcome: Outcome::Ok(value.clone()),
                order: 0,
            },
        );
        value
    }

    /// For `select!`: which branch won on the run that is being replayed, if there is a record.
    pub(crate) fn recorded_winner(&self, spec: &StepSpec) -> Option<usize> {
        let record = self.find_record("race", spec)?;
        match &record.outcome {
            Outcome::Ok(winner) => {
                let winner = winner
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())?;
                self.apply_signals_before(record.order);
                Some(winner)
            }
            Outcome::Fault(_) | Outcome::Timeout(_) => None,
        }
    }

    pub(crate) fn record_winner(&self, spec: &StepSpec, winner: usize) {
        lock(&self.journal).insert(
            self.replay,
            spec.seq,
            StepRecord {
                kind: "race".to_string(),
                name: "race".to_string(),
                ordinal: spec.ordinal,
                input_hash: String::new(),
                call_site: site(spec.location),
                outcome: Outcome::Ok(json!(winner)),
                order: 0,
            },
        );
    }

    /// `ctx.patched(id)`: true for new executions, false when replaying a history that started before the patch.
    pub(crate) fn patched(&self, id: &str) -> bool {
        let mut journal = lock(&self.journal);
        if let Some(known) = journal.patches.get(id) {
            return *known;
        }
        let replaying_old_history =
            self.boundary > 0 && self.next_seq.load(Ordering::SeqCst) <= self.boundary;
        let applies = !replaying_old_history;
        journal.patches.insert(id.to_string(), applies);
        applies
    }

    // ---- signals in replay ----

    fn apply_signal(&self, signal: &SignalRecord) {
        if let Some(dispatch) = self.dispatch.get()
            && let Err(error) = (dispatch.signal)(&signal.name, signal.argument.clone())
        {
            tracing::warn!(%error, signal = %signal.name, "a recorded signal could not be re-applied");
        }
        self.state_changed.notify_waiters();
    }

    /// Applies the recorded signals that arrived before history position `order`.
    fn apply_signals_before(&self, order: u64) {
        loop {
            let next = {
                let mut pending = lock(&self.pending_signals);
                match pending.front() {
                    Some(signal) if signal.order < order => pending.pop_front(),
                    Some(_) | None => None,
                }
            };
            match next {
                Some(signal) => self.apply_signal(&signal),
                None => return,
            }
        }
    }

    /// Applies every recorded signal: the instance has reached the end of its history.
    fn apply_all_pending(&self) {
        self.apply_signals_before(u64::MAX);
    }

    /// Applies the next recorded signal, if any. A replaying `wait_condition` uses it to move through history.
    pub(crate) fn apply_next_pending(&self) -> bool {
        let next = lock(&self.pending_signals).pop_front();
        match next {
            Some(signal) => {
                self.apply_signal(&signal);
                true
            }
            None => false,
        }
    }

    // ---- events and persistence ----

    pub(crate) async fn emit(&self, event: Event) -> Result<(), Error> {
        if !self.ephemeral
            && self.replay_only.is_none()
            && let Some(stored) = event.to_stored()
        {
            let shared = self.shared()?;
            shared.persistence.save_event(stored).await?;
        }
        if self.replay_only.is_none() {
            self.log.push(event);
        }
        Ok(())
    }

    /// Writes the journal to the store. A failure here means the instance is no longer durable, so it is returned.
    pub(crate) async fn persist(&self) -> Result<(), Error> {
        if self.ephemeral || self.replay_only.is_some() {
            return Ok(());
        }
        let shared = self.shared()?;
        let _guard = self.persist_lock.lock().await;
        let snapshot = {
            let mut journal = lock(&self.journal);
            journal.updated_at = Some(self.clock.now());
            journal.clone()
        };
        let current_task = snapshot
            .steps
            .values()
            .next_back()
            .map(|record| record.name.clone())
            .unwrap_or_default();
        let data = serde_json::to_value(&snapshot)
            .map_err(|e| Error::Engine(format!("the journal could not be serialized: {e}")))?;
        let checkpoint = WorkflowCheckpoint {
            instance_id: self.id.clone(),
            current_task,
            data: json!({ "jackdaw_native": data }),
            timestamp: Utc::now(),
        };
        shared.persistence.save_checkpoint(checkpoint).await?;
        Ok(())
    }

    // ---- signals, queries and events from outside ----

    async fn dispatch(&self) -> Result<&Dispatch, Error> {
        loop {
            let ready = self.dispatch_ready.notified();
            tokio::pin!(ready);
            ready.as_mut().enable();
            if let Some(dispatch) = self.dispatch.get() {
                return Ok(dispatch);
            }
            if self.status() != JournalStatus::Running || self.has_ended() {
                return Err(self.not_running());
            }
            ready.await;
        }
    }

    fn not_running(&self) -> Error {
        Error::invalid(format!(
            "workflow instance `{}` is not running (it is {:?})",
            self.id,
            self.status()
        ))
    }

    pub(crate) async fn deliver_signal(&self, name: &str, argument: Value) -> Result<(), Error> {
        if self.status() != JournalStatus::Running || self.has_ended() {
            return Err(self.not_running());
        }
        let dispatch = self.dispatch().await?;
        let replaying = !lock(&self.pending_signals).is_empty();
        let record = {
            let mut journal = lock(&self.journal);
            SignalRecord {
                name: name.to_string(),
                argument: argument.clone(),
                order: journal.next_order(),
            }
        };
        if replaying {
            // Older signals have not been re-applied yet; this one goes after them.
            lock(&self.pending_signals).push_back(record.clone());
        } else {
            (dispatch.signal)(name, argument)?;
        }
        lock(&self.journal).signals.push(record);
        self.state_changed.notify_waiters();
        self.persist().await
    }

    pub(crate) async fn query(&self, name: &str) -> Result<Value, Error> {
        (self.dispatch().await?.query)(name)
    }

    /// Stores an event for a `wait_for_event`. It is persisted before this returns, so it survives a crash.
    pub(crate) async fn deliver_event(&self, name: &str, payload: Value) -> Result<(), Error> {
        if self.status() != JournalStatus::Running || self.has_ended() {
            return Err(self.not_running());
        }
        lock(&self.journal)
            .inbox
            .entry(name.to_string())
            .or_default()
            .push_back(payload);
        self.inbox_changed.notify_waiters();
        self.persist().await
    }

    pub(crate) fn request_cancel(&self) {
        self.cancel.send_replace(true);
    }

    // ---- running steps ----

    fn lookup(&self, spec: &StepSpec, hash: &str) -> Result<Option<StepRecord>, Error> {
        let journal = lock(&self.journal);
        match self.replay {
            ReplayMode::Strict => match journal.steps.get(&spec.seq) {
                None => Ok(None),
                Some(record)
                    if record.kind == spec.kind
                        && record.name == spec.name
                        && record.input_hash == hash =>
                {
                    Ok(Some(record.clone()))
                }
                Some(record) => {
                    Err(self.divergence(spec, Some(format!("{} {}", record.kind, record.name))))
                }
            },
            ReplayMode::Lenient => Ok(journal
                .steps
                .values()
                .find(|record| {
                    record.kind == spec.kind
                        && record.name == spec.name
                        && record.ordinal == spec.ordinal
                })
                .cloned()),
        }
    }

    fn divergence(&self, spec: &StepSpec, recorded: Option<String>) -> Error {
        Error::NonDeterminism(Box::new(Divergence {
            instance_id: self.id.clone(),
            workflow: self.workflow.to_string(),
            step: spec.seq,
            expected: format!("{} {}", spec.kind, spec.name),
            recorded,
            call_site: site(spec.location),
        }))
    }

    fn event_task_name(spec: &StepSpec) -> Option<String> {
        matches!(spec.kind, "call" | "run").then(|| spec.name.clone())
    }

    pub(crate) async fn run_step(self: &Arc<Self>, spec: StepSpec) -> Result<Value, StepError> {
        let hash = hash_of(&spec.input);
        if let Some(record) = self.lookup(&spec, &hash)? {
            self.apply_signals_before(record.order);
            self.emit_step_events(&spec, &record.outcome, true).await?;
            return record.outcome.into_result();
        }
        if let Some(replay_only) = &self.replay_only {
            return if replay_only.allow_frontier {
                self.frontier.notify_one();
                std::future::pending().await
            } else {
                Err(self.divergence(&spec, None).into())
            };
        }
        // The history ends here: whatever arrived after its last record has happened by now.
        self.apply_all_pending();

        if let Some(task_name) = Self::event_task_name(&spec) {
            self.emit(Event::TaskStarted {
                instance_id: self.id.clone(),
                task_name,
                replayed: false,
                timestamp: self.clock.now(),
            })
            .await?;
        }
        let key = self.step_key(&spec);
        let outcome = match spec.timeout {
            Some(limit) => {
                let deadline = self.deadline(format!("timeout:{key}"), limit).await?;
                let clock = Arc::clone(&self.clock);
                tokio::select! {
                    outcome = self.execute_live(&spec, &key) => outcome?,
                    () = clock.sleep_until(deadline) => Outcome::Timeout(format!(
                        "{} `{}` exceeded its timeout of {limit:?}",
                        step_noun(spec.kind),
                        spec.name
                    )),
                }
            }
            None => self.execute_live(&spec, &key).await?,
        };
        {
            let mut journal = lock(&self.journal);
            journal.deadlines.remove(&format!("timeout:{key}"));
            journal.deadlines.remove(&format!("sleep:{key}"));
            if spec.kind == "event" && matches!(outcome, Outcome::Ok(_)) {
                // The event moves from the inbox into the step's record in one write.
                if let Some(queue) = journal.inbox.get_mut(&spec.name) {
                    queue.pop_front();
                    if queue.is_empty() {
                        journal.inbox.remove(&spec.name);
                    }
                }
                let mut claims = lock(&self.claims);
                if let Some(claimed) = claims.get_mut(&spec.name) {
                    *claimed = claimed.saturating_sub(1);
                }
            }
            journal.insert(
                self.replay,
                spec.seq,
                StepRecord {
                    kind: spec.kind.to_string(),
                    name: spec.name.clone(),
                    ordinal: spec.ordinal,
                    input_hash: hash,
                    call_site: site(spec.location),
                    outcome: outcome.clone(),
                    order: 0,
                },
            );
        }
        self.persist().await?;
        self.emit_step_events(&spec, &outcome, false).await?;
        outcome.into_result()
    }

    /// The durable deadline stored under `key`, recording `now + duration` the first time.
    async fn deadline(&self, key: String, duration: Duration) -> Result<DateTime<Utc>, Error> {
        let existing = lock(&self.journal).deadlines.get(&key).copied();
        if let Some(deadline) = existing {
            return Ok(deadline);
        }
        let deadline = super::clock::after(self.clock.now(), duration);
        lock(&self.journal).deadlines.insert(key, deadline);
        self.persist().await?;
        Ok(deadline)
    }

    async fn emit_step_events(
        &self,
        spec: &StepSpec,
        outcome: &Outcome,
        replayed: bool,
    ) -> Result<(), Error> {
        let Some(task_name) = Self::event_task_name(spec) else {
            return Ok(());
        };
        let now = self.clock.now();
        if replayed {
            self.emit(Event::TaskStarted {
                instance_id: self.id.clone(),
                task_name: task_name.clone(),
                replayed: true,
                timestamp: now,
            })
            .await?;
        }
        let event = match outcome {
            Outcome::Ok(result) => Event::TaskCompleted {
                instance_id: self.id.clone(),
                task_name,
                result: result.clone(),
                replayed,
                timestamp: now,
            },
            Outcome::Fault(fault) => Event::TaskFaulted {
                instance_id: self.id.clone(),
                task_name,
                error: fault.to_string(),
                replayed,
                timestamp: now,
            },
            Outcome::Timeout(detail) => Event::TaskFaulted {
                instance_id: self.id.clone(),
                task_name,
                error: detail.clone(),
                replayed,
                timestamp: now,
            },
        };
        self.emit(event).await
    }

    async fn execute_live(self: &Arc<Self>, spec: &StepSpec, key: &str) -> Result<Outcome, Error> {
        match &spec.exec {
            StepExec::Task {
                target: TaskTarget::Native(name),
                options,
            } => self.run_native(spec, name, options).await,
            StepExec::Task {
                target: TaskTarget::Catalog { name, version },
                ..
            } => {
                let shared = self.shared()?;
                let definition = shared.catalogs.load(name, version).await?;
                Ok(into_outcome(
                    shared.run_definition(&definition, spec.input.clone()).await,
                ))
            }
            StepExec::Child(target) => {
                let shared = self.shared()?;
                let result = match target {
                    RunTarget::Native(info) => {
                        shared
                            .run_native_child(
                                &self.id,
                                format!("{}/{}", self.id, spec.seq),
                                &info.id,
                                spec.input.clone(),
                            )
                            .await
                    }
                    RunTarget::Definition(definition) => {
                        shared.run_definition(definition, spec.input.clone()).await
                    }
                    RunTarget::Catalog { name, version } => {
                        let definition = shared.catalogs.load(name, version).await?;
                        shared.run_definition(&definition, spec.input.clone()).await
                    }
                };
                Ok(into_outcome(result))
            }
            StepExec::Sleep(duration) => {
                let deadline = self.deadline(format!("sleep:{key}"), *duration).await?;
                self.clock.sleep_until(deadline).await;
                Ok(Outcome::Ok(Value::Null))
            }
            StepExec::Event => loop {
                let arrived = self.inbox_changed.notified();
                tokio::pin!(arrived);
                arrived.as_mut().enable();
                {
                    let journal = lock(&self.journal);
                    let mut claims = lock(&self.claims);
                    let claimed = claims.entry(spec.name.clone()).or_insert(0);
                    if let Some(payload) = journal
                        .inbox
                        .get(&spec.name)
                        .and_then(|queue| queue.get(*claimed))
                    {
                        *claimed += 1;
                        return Ok(Outcome::Ok(payload.clone()));
                    }
                }
                arrived.await;
            },
        }
    }

    async fn run_native(
        self: &Arc<Self>,
        spec: &StepSpec,
        name: &str,
        options: &TaskOptions,
    ) -> Result<Outcome, Error> {
        let shared = self.shared()?;
        let task = shared
            .registry
            .tasks
            .get(name)
            .ok_or_else(|| Error::invalid(format!("no task named `{name}` is registered")))?
            .clone();
        let cache_key = compute_cache_key(name, &spec.input);
        if task.cached
            && let Some(hit) = shared.cache.get(&cache_key).await?
        {
            return Ok(Outcome::Ok(hit.output));
        }
        let attempts = options.retry.as_ref().map_or(1, Retry::attempts);
        let mut attempt = 1;
        loop {
            let context =
                TaskContext::new(&self.id, name, attempt, format!("{}/{}", self.id, spec.seq));
            match task.invoke(context, spec.input.clone()).await {
                Ok(value) => {
                    if task.cached {
                        shared
                            .cache
                            .set(CacheEntry {
                                key: cache_key,
                                inputs: spec.input.clone(),
                                output: value.clone(),
                                timestamp: Utc::now(),
                            })
                            .await?;
                    }
                    return Ok(Outcome::Ok(value));
                }
                Err(error) if error.retryable && attempt < attempts => {
                    self.emit(Event::TaskRetried {
                        instance_id: self.id.clone(),
                        task_name: name.to_string(),
                        attempt,
                        timestamp: self.clock.now(),
                    })
                    .await?;
                    let delay = error
                        .next_retry_delay
                        .or_else(|| {
                            options
                                .retry
                                .as_ref()
                                .map(|retry| retry.delay_after(attempt))
                        })
                        .unwrap_or_default();
                    super::clock::sleep(self.clock.as_ref(), delay).await;
                    attempt += 1;
                }
                Err(error) => return Ok(Outcome::Fault(*error.fault)),
            }
        }
    }
}

fn step_noun(kind: &str) -> &'static str {
    match kind {
        "call" => "task",
        "run" => "child workflow",
        "sleep" => "sleep",
        "event" => "wait for event",
        _ => "step",
    }
}

fn into_outcome(result: Result<Value, Error>) -> Outcome {
    match result {
        Ok(value) => Outcome::Ok(value),
        Err(Error::Faulted(fault)) => Outcome::Fault(fault),
        Err(Error::Timeout { detail }) => Outcome::Timeout(detail),
        Err(other) => Outcome::Fault(Fault {
            kind: error_types::RUNTIME.to_string(),
            status: None,
            title: None,
            detail: Some(other.to_string()),
            instance: None,
        }),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Driving a workflow
// ---------------------------------------------------------------------------------------------------------------

enum Ended<T> {
    Finished(T),
    Cancelled,
    Frontier,
}

async fn cancelled(mut cancel: watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Runs workflow `W` for `core` until it finishes, is cancelled, or (when only replaying) reaches the end of its history.
pub(crate) async fn drive<W: WorkflowDef>(
    core: Arc<InstanceCore>,
    input: Value,
) -> Result<Value, Error> {
    let view = WorkflowContextView::new(&core.id);
    let state = Arc::new(Mutex::new(W::init(&view)));

    let signals: HashMap<&'static str, _> = W::signals()
        .into_iter()
        .map(|handler| (handler.name, handler))
        .collect();
    let queries: HashMap<&'static str, _> = W::queries()
        .into_iter()
        .map(|handler| (handler.name, handler))
        .collect();
    let (signal_state, query_state) = (Arc::clone(&state), Arc::clone(&state));
    let (signal_id, query_id) = (core.id.clone(), core.id.clone());
    let dispatch = Dispatch {
        signal: Box::new(move |name, argument| {
            let handler = signals
                .get(name)
                .ok_or_else(|| Error::invalid(format!("workflow has no signal named `{name}`")))?;
            let mut state = lock(&signal_state);
            let mut context = SyncWorkflowContext::<W>::new(&signal_id);
            (handler.apply)(&mut state, &mut context, argument).map_err(Error::invalid)
        }),
        query: Box::new(move |name| {
            let handler = queries
                .get(name)
                .ok_or_else(|| Error::invalid(format!("workflow has no query named `{name}`")))?;
            let view = WorkflowContextView::new(&query_id);
            (handler.read)(&lock(&query_state), &view).map_err(Error::Engine)
        }),
    };
    let _ = core.dispatch.set(dispatch);
    core.dispatch_ready.notify_waiters();

    // Signals from journals written before signals were ordered are applied up front, as they always were. Ordered
    // signals are applied as replay reaches the point in history where they arrived.
    let unordered: Vec<SignalRecord> = lock(&core.journal)
        .signals
        .iter()
        .filter(|signal| signal.order == 0)
        .cloned()
        .collect();
    if let Some(dispatch) = core.dispatch.get() {
        for signal in unordered {
            (dispatch.signal)(&signal.name, signal.argument)?;
        }
    }

    let typed: W::In = serde_json::from_value(input).map_err(|e| {
        Error::Faulted(Fault::validation(format!(
            "the workflow's input does not match its type: {e}"
        )))
    })?;

    let resumed = core.boundary > 0;
    core.emit(if resumed {
        Event::WorkflowResumed {
            instance_id: core.id.clone(),
            timestamp: core.clock.now(),
        }
    } else {
        Event::WorkflowStarted {
            instance_id: core.id.clone(),
            workflow: core.workflow.to_string(),
            input: lock(&core.journal).input.clone(),
            timestamp: core.clock.now(),
        }
    })
    .await?;

    let mut context = WorkflowContext::<W>::new(Arc::clone(&core), state);
    let frontier_possible = core
        .replay_only
        .as_ref()
        .is_some_and(|replay| replay.allow_frontier);
    let ended = tokio::select! {
        result = W::run(&mut context, typed) => Ended::Finished(result),
        () = cancelled(core.cancel.subscribe()) => Ended::Cancelled,
        () = core.frontier.notified(), if frontier_possible => Ended::Frontier,
    };

    let mut result = match ended {
        Ended::Finished(Ok(output)) => serde_json::to_value(output).map_err(|e| {
            Error::Engine(format!(
                "the workflow's output could not be serialized: {e}"
            ))
        }),
        Ended::Finished(Err(error)) => Err(Error::from(error)),
        Ended::Cancelled => Err(Error::Cancelled),
        Ended::Frontier => Ok(Value::Null),
    };
    if core.replay_only.is_some() {
        return result;
    }

    core.finish(&result);
    if let Err(error) = core.persist().await {
        // The end could not be recorded: report that instead, the instance will be resumed.
        result = Err(error);
    }
    let terminal = match &result {
        Ok(output) => Event::WorkflowCompleted {
            instance_id: core.id.clone(),
            output: output.clone(),
            timestamp: core.clock.now(),
        },
        Err(Error::Cancelled) => Event::WorkflowCancelled {
            instance_id: core.id.clone(),
            timestamp: core.clock.now(),
        },
        Err(error) => Event::WorkflowFailed {
            instance_id: core.id.clone(),
            error: error.to_string(),
            timestamp: core.clock.now(),
        },
    };
    if let Err(error) = core.emit_terminal(terminal).await
        && result.is_ok()
    {
        result = Err(error);
    }
    result
}

impl InstanceCore {
    /// Emits a terminal event and closes the stream after it.
    async fn emit_terminal(&self, event: Event) -> Result<(), Error> {
        let emitted = self.emit(event).await;
        self.log.close();
        emitted
    }

    /// Waits until `condition` holds for the workflow's state. While replaying, recorded signals are applied one at a
    /// time until it does, which is how the original run got past this point.
    pub(crate) async fn wait_until(
        &self,
        condition: &(dyn Fn() -> bool + Send + Sync),
        location: &'static Location<'static>,
    ) -> Result<(), StepError> {
        loop {
            let changed = self.state_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if condition() {
                return Ok(());
            }
            if self.apply_next_pending() {
                continue;
            }
            if let Some(replay_only) = &self.replay_only {
                // No signal is left to apply, so nothing in the history ever satisfied the condition.
                if replay_only.allow_frontier {
                    self.frontier.notify_one();
                    return std::future::pending().await;
                }
                return Err(Error::NonDeterminism(Box::new(Divergence {
                    instance_id: self.id.clone(),
                    workflow: self.workflow.to_string(),
                    step: self.next_seq.load(Ordering::SeqCst),
                    expected: "a signal that satisfies wait_condition".to_string(),
                    recorded: None,
                    call_site: site(location),
                }))
                .into());
            }
            changed.await;
        }
    }
}

/// What `Engine::replay_all` does for one stored instance: replay its journal and see whether the code still agrees.
pub(crate) async fn replay_instance(
    shared: &Arc<Shared>,
    id: &str,
) -> Result<Option<Divergence>, Error> {
    let Some(journal) = shared.load_journal(id).await? else {
        return Ok(None);
    };
    let Ok(entry) = shared.find_workflow(&journal.workflow) else {
        return Ok(None); // not a workflow this engine knows
    };
    let core = InstanceCore::new(
        id.to_string(),
        entry.id.clone(),
        entry.replay,
        Arc::downgrade(shared),
        journal.clone(),
        Some(ReplayOnly {
            allow_frontier: true,
        }),
        true,
    );
    match entry.runner.spawn(core, journal.input).await {
        Ok(_) => Ok(None),
        Err(Error::NonDeterminism(divergence)) => Ok(Some(*divergence)),
        Err(other) => Err(other),
    }
}
