//! Workflows: the traits the `#[workflow]` macros implement, the typed markers, and `Workflow<In, Out>` for YAML.
//!
//! Write workflows with the `workflow` and `workflow_methods` attributes; see the
//! [workflows chapter](crate::guide::workflows). Workflows defined in YAML are loaded with [`Workflow`].
use super::catalog::CatalogWorkflowRef;
use super::context::WorkflowContext;
use super::error::{Error, WorkflowResult};
use super::runtime::{Registry, register_workflow};
use super::task::{BoxFuture, TaskDef, TaskTarget};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serverless_workflow_core::models::workflow::WorkflowDefinition;
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

/// `namespace`, `name` and `version`: the identity of a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId {
    /// Groups related workflows.
    pub namespace: String,
    /// The workflow's name within its namespace.
    pub name: String,
    /// The workflow's version.
    pub version: String,
}

impl WorkflowId {
    /// The identity declared by `#[workflow(..)]` on `W`.
    #[must_use]
    pub fn of<W: WorkflowMeta>() -> Self {
        Self {
            namespace: W::NAMESPACE.to_string(),
            name: W::NAME.to_string(),
            version: W::VERSION.to_string(),
        }
    }

    pub(crate) fn key(&self) -> String {
        format!("{}/{}/{}", self.namespace, self.name, self.version)
    }
}

impl fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}@{}", self.namespace, self.name, self.version)
    }
}

/// How strictly a restarted instance is matched against its history.
///
/// Chosen per workflow with `#[workflow(replay = "lenient")]`. [The durability chapter](crate::guide::durability)
/// explains when to loosen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// Steps are matched by position, kind, name and input. Any difference is `Error::NonDeterminism`. The default.
    Strict,
    /// Steps are matched by kind, name and how many times that name has been used. Inserted steps run live.
    Lenient,
}

/// Implemented by `#[workflow]`.
pub trait WorkflowMeta {
    /// The namespace from `#[workflow(namespace = ..)]`.
    const NAMESPACE: &'static str;
    /// The name from `#[workflow(name = ..)]`.
    const NAME: &'static str;
    /// The version from `#[workflow(version = ..)]`.
    const VERSION: &'static str;
    /// The replay mode; strict unless the attribute says otherwise.
    const REPLAY: ReplayMode;
}

/// Implemented by `#[workflow_methods]`.
pub trait WorkflowDef: WorkflowMeta + Sized + Send + Sync + 'static {
    /// The `#[run]` method's input.
    type In: Serialize + DeserializeOwned + Send + 'static;
    /// The `#[run]` method's output.
    type Out: Serialize + DeserializeOwned + Send + 'static;

    /// Builds the instance's state (the `#[init]` method, or `Default`).
    fn init(ctx: &WorkflowContextView) -> Self;

    /// The `#[run]` method.
    fn run<'a>(
        ctx: &'a mut WorkflowContext<Self>,
        input: Self::In,
    ) -> BoxFuture<'a, WorkflowResult<Self::Out>>;

    /// The `#[signal]` methods.
    fn signals() -> Vec<SignalHandler<Self>>;

    /// The `#[query]` methods.
    fn queries() -> Vec<QueryHandler<Self>>;

    /// The `#[update]` methods.
    #[must_use]
    fn updates() -> Vec<UpdateHandler<Self>> {
        Vec::new()
    }

    /// The tasks and workflows `#[run]` refers to, found by the macro. Checked when the engine is built.
    fn dependencies() -> Vec<Dependency>;
}

/// A workflow's `#[run]` method as a value (`OrderWorkflow::run`).
pub struct WorkflowRef<W, I, O> {
    _types: PhantomData<fn(W, I) -> O>,
}

impl<W, I, O> WorkflowRef<W, I, O> {
    /// The marker for workflow `W`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _types: PhantomData,
        }
    }
}

impl<W, I, O> Default for WorkflowRef<W, I, O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W, I, O> Clone for WorkflowRef<W, I, O> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W, I, O> Copy for WorkflowRef<W, I, O> {}

/// A signal handler as a value (`Approval::approve`).
pub struct SignalRef<W, A> {
    pub(crate) name: &'static str,
    _types: PhantomData<fn(W, A)>,
}

impl<W, A> SignalRef<W, A> {
    /// The marker for the signal `name`.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _types: PhantomData,
        }
    }
}

impl<W, A> Clone for SignalRef<W, A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W, A> Copy for SignalRef<W, A> {}

/// A query handler as a value (`Approval::status`).
pub struct QueryRef<W, R> {
    pub(crate) name: &'static str,
    _types: PhantomData<fn(W) -> R>,
}

impl<W, R> QueryRef<W, R> {
    /// The marker for the query `name`.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _types: PhantomData,
        }
    }
}

impl<W, R> Clone for QueryRef<W, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W, R> Copy for QueryRef<W, R> {}

type SignalFn<W> = Box<
    dyn Fn(&mut W, &mut SyncWorkflowContext<W>, serde_json::Value) -> Result<(), String>
        + Send
        + Sync,
>;
type UpdateFn<W> = Box<
    dyn Fn(
            &mut W,
            &mut SyncWorkflowContext<W>,
            serde_json::Value,
        ) -> Result<Result<serde_json::Value, super::error::UpdateError>, String>
        + Send
        + Sync,
>;
type QueryFn<W> =
    Box<dyn Fn(&W, &WorkflowContextView) -> Result<serde_json::Value, String> + Send + Sync>;

/// A `#[signal]` method, with its argument type erased.
pub struct SignalHandler<W> {
    pub(crate) name: &'static str,
    pub(crate) apply: SignalFn<W>,
}

impl<W> SignalHandler<W> {
    /// Wraps `handler`, decoding the signal's JSON payload into `A`.
    pub fn new<A, F>(name: &'static str, handler: F) -> Self
    where
        A: DeserializeOwned + 'static,
        F: Fn(&mut W, &mut SyncWorkflowContext<W>, A) + Send + Sync + 'static,
    {
        Self {
            name,
            apply: Box::new(move |state, ctx, value| {
                let arg: A = serde_json::from_value(value)
                    .map_err(|e| format!("invalid signal argument: {e}"))?;
                handler(state, ctx, arg);
                Ok(())
            }),
        }
    }
}

/// A workflow's `#[update]` method as a value (`Cart::add_item`): a signal that answers the caller.
pub struct UpdateRef<W, A, R> {
    pub(crate) name: &'static str,
    _types: PhantomData<fn(W, A) -> R>,
}

impl<W, A, R> UpdateRef<W, A, R> {
    /// The marker for the update `name`.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _types: PhantomData,
        }
    }
}

impl<W, A, R> Clone for UpdateRef<W, A, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W, A, R> Copy for UpdateRef<W, A, R> {}

/// An `#[update]` method, with its argument and result types erased.
pub struct UpdateHandler<W> {
    pub(crate) name: &'static str,
    pub(crate) apply: UpdateFn<W>,
}

impl<W> UpdateHandler<W> {
    /// Wraps `handler`, decoding the argument into `A` and encoding the result.
    pub fn new<A, R, F>(name: &'static str, handler: F) -> Self
    where
        A: DeserializeOwned + 'static,
        R: Serialize + 'static,
        F: Fn(&mut W, &mut SyncWorkflowContext<W>, A) -> Result<R, super::error::UpdateError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            name,
            apply: Box::new(move |state, ctx, value| {
                let arg: A = serde_json::from_value(value)
                    .map_err(|e| format!("invalid update argument: {e}"))?;
                Ok(match handler(state, ctx, arg) {
                    Ok(result) => Ok(serde_json::to_value(result)
                        .map_err(|e| format!("the update result could not be serialized: {e}"))?),
                    Err(rejection) => Err(rejection),
                })
            }),
        }
    }
}

/// A `#[query]` method, with its result type erased.
pub struct QueryHandler<W> {
    pub(crate) name: &'static str,
    pub(crate) read: QueryFn<W>,
}

impl<W> QueryHandler<W> {
    /// Wraps `handler`, encoding its result as JSON.
    pub fn new<R, F>(name: &'static str, handler: F) -> Self
    where
        R: Serialize + 'static,
        F: Fn(&W, &WorkflowContextView) -> R + Send + Sync + 'static,
    {
        Self {
            name,
            read: Box::new(move |state, ctx| {
                serde_json::to_value(handler(state, ctx))
                    .map_err(|e| format!("the query result could not be serialized: {e}"))
            }),
        }
    }
}

/// What `#[init]` and `#[query]` methods can see of the instance.
#[derive(Debug, Clone)]
pub struct WorkflowContextView {
    instance_id: String,
}

impl WorkflowContextView {
    pub(crate) fn new(instance_id: &str) -> Self {
        Self {
            instance_id: instance_id.to_string(),
        }
    }

    /// The id of the running instance.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

/// What `#[signal]` methods can see of the instance.
#[derive(Debug, Clone)]
pub struct SyncWorkflowContext<W> {
    instance_id: String,
    _workflow: PhantomData<fn() -> W>,
}

impl<W> SyncWorkflowContext<W> {
    pub(crate) fn new(instance_id: &str) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            _workflow: PhantomData,
        }
    }

    /// The id of the running instance.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

/// What a workflow needs from the engine, found by scanning its `#[run]` body.
pub enum Dependency {
    /// A task, native or from a catalog.
    Task(TaskTarget),
    /// A native child workflow.
    Workflow(WorkflowInfo),
    /// A child workflow from a catalog.
    CatalogWorkflow {
        /// The entry's name in the catalog.
        name: &'static str,
        /// The entry's version in the catalog.
        version: &'static str,
    },
}

/// A native workflow, with the function that registers it (and, through its own dependencies, its children).
#[derive(Clone)]
pub struct WorkflowInfo {
    /// The workflow's identity.
    pub id: WorkflowId,
    /// Adds the workflow and its dependencies to a registry.
    pub register: fn(&mut Registry),
}

/// Anything `ctx.call` or `ctx.run` accepts, as seen by the dependency scan.
pub trait AsDependency {
    /// What the engine needs to have registered for this to run.
    fn dependency(&self) -> Dependency;
}

impl<T: TaskDef> AsDependency for T {
    fn dependency(&self) -> Dependency {
        Dependency::Task(self.target())
    }
}

impl<W, I, O> AsDependency for WorkflowRef<W, I, O>
where
    W: WorkflowDef<In = I, Out = O>,
{
    fn dependency(&self) -> Dependency {
        Dependency::Workflow(WorkflowInfo {
            id: WorkflowId::of::<W>(),
            register: register_workflow::<W>,
        })
    }
}

impl<I, O> AsDependency for CatalogWorkflowRef<I, O> {
    fn dependency(&self) -> Dependency {
        Dependency::CatalogWorkflow {
            name: self.name,
            version: self.version,
        }
    }
}

/// Where a run request points.
pub enum RunTarget {
    /// A native workflow.
    Native(WorkflowInfo),
    /// A workflow loaded from YAML or JSON.
    Definition(Arc<WorkflowDefinition>),
    /// A workflow from a catalog.
    Catalog {
        /// The entry's name in the catalog.
        name: &'static str,
        /// The entry's version in the catalog.
        version: &'static str,
    },
}

/// Anything `Engine::execute` and `ctx.run` accept: a workflow's run marker, a YAML workflow, or a catalog workflow.
pub trait Runnable {
    /// What the workflow takes.
    type In: Serialize + Send + 'static;
    /// What the workflow returns.
    type Out: DeserializeOwned + Send + 'static;

    /// Where the workflow comes from.
    fn target(self) -> RunTarget;
}

impl<W, I, O> Runnable for WorkflowRef<W, I, O>
where
    W: WorkflowDef<In = I, Out = O>,
    I: Serialize + Send + 'static,
    O: DeserializeOwned + Send + 'static,
{
    type In = I;
    type Out = O;

    fn target(self) -> RunTarget {
        RunTarget::Native(WorkflowInfo {
            id: WorkflowId::of::<W>(),
            register: register_workflow::<W>,
        })
    }
}

impl<I, O> Runnable for &Workflow<I, O>
where
    I: Serialize + Send + 'static,
    O: DeserializeOwned + Send + 'static,
{
    type In = I;
    type Out = O;

    fn target(self) -> RunTarget {
        RunTarget::Definition(Arc::clone(&self.definition))
    }
}

impl<I, O> Runnable for CatalogWorkflowRef<I, O>
where
    I: Serialize + Send + 'static,
    O: DeserializeOwned + Send + 'static,
{
    type In = I;
    type Out = O;

    fn target(self) -> RunTarget {
        RunTarget::Catalog {
            name: self.name,
            version: self.version,
        }
    }
}

/// A workflow defined in YAML or JSON, bound to the Rust types of its input and output.
///
/// There is no untyped form and no default for `In` and `Out`. Code that cannot know the shape writes
/// `Workflow::<serde_json::Value, serde_json::Value>`.
///
/// ```
/// use jackdaw::prelude::*;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct Greeting {
///     name: String,
/// }
///
/// let workflow = Workflow::<Greeting, serde_json::Value>::from_yaml(
///     r#"
/// document:
///   dsl: '1.0.0'
///   namespace: examples
///   name: greet
///   version: '0.1.0'
/// do:
///   - say:
///       set:
///         message: hello
/// "#,
/// )?;
/// assert_eq!(workflow.name(), "greet");
/// # Ok::<(), Error>(())
/// ```
pub struct Workflow<In, Out> {
    definition: Arc<WorkflowDefinition>,
    _types: PhantomData<fn(In) -> Out>,
}

impl<In, Out> Clone for Workflow<In, Out> {
    fn clone(&self) -> Self {
        Self {
            definition: Arc::clone(&self.definition),
            _types: PhantomData,
        }
    }
}

impl<In, Out> fmt::Debug for Workflow<In, Out> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workflow")
            .field("namespace", &self.namespace())
            .field("name", &self.name())
            .field("version", &self.version())
            .finish()
    }
}

impl<In, Out> Workflow<In, Out> {
    /// Parses a YAML workflow definition.
    ///
    /// # Errors
    /// `Error::Invalid` if the text is not a valid workflow definition.
    pub fn from_yaml(yaml: &str) -> Result<Self, Error> {
        serde_yaml::from_str::<WorkflowDefinition>(yaml)
            .map(Self::from_definition)
            .map_err(|e| Error::invalid(format!("not a valid workflow definition: {e}")))
    }

    /// Parses a JSON workflow definition.
    ///
    /// # Errors
    /// `Error::Invalid` if the text is not a valid workflow definition.
    pub fn from_json(json: &str) -> Result<Self, Error> {
        serde_json::from_str::<WorkflowDefinition>(json)
            .map(Self::from_definition)
            .map_err(|e| Error::invalid(format!("not a valid workflow definition: {e}")))
    }

    /// Reads a `.yaml`/`.yml` or `.json` file.
    ///
    /// # Errors
    /// `Error::Invalid` if the file cannot be read or is not a valid workflow definition.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::invalid(format!("cannot read {}: {e}", path.display())))?;
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            Self::from_json(&text)
        } else {
            Self::from_yaml(&text)
        }
    }

    /// Wraps an already parsed definition.
    #[must_use]
    pub fn from_definition(definition: WorkflowDefinition) -> Self {
        Self {
            definition: Arc::new(definition),
            _types: PhantomData,
        }
    }

    /// The parsed definition.
    #[must_use]
    pub fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }

    /// The document's namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.definition.document.namespace
    }

    /// The document's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.definition.document.name
    }

    /// The document's version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.definition.document.version
    }
}
