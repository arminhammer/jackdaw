//! Interactive pipeline session for notebook-driven workflow development.
//!
//! A [`Session`] lets you add and immediately execute one workflow step at a
//! time, accumulating the native `TaskDefinition` structs as you go. When you
//! are satisfied you can export the assembled [`WorkflowDefinition`] or its
//! YAML serialisation and hand it off to the full jackdaw engine.

use crate::{builder::DurableEngineBuilder, durableengine::DurableEngine};
use serverless_workflow_core::models::{
    task::TaskDefinition,
    workflow::{WorkflowDefinition, WorkflowDefinitionMetadata},
    map::Map,
};
use snafu::Snafu;
use std::{sync::Arc, time::Duration};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Engine error: {message}"))]
    Engine { message: String },

    #[snafu(display("YAML error: {message}"))]
    Yaml { message: String },

    #[snafu(display("Runtime error: {message}"))]
    Runtime { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Interactive pipeline session.
///
/// Accumulates steps as native [`TaskDefinition`] values so the exported
/// [`WorkflowDefinition`] is built from real Rust data structures, not
/// re-parsed strings.
///
/// # Example (Rust)
///
/// ```no_run
/// use jackdaw::session::Session;
/// use serverless_workflow_core::models::workflow::WorkflowDefinition;
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let step_yaml = std::fs::read_to_string("step.yaml")?;
/// let step_wf: WorkflowDefinition = serde_yaml::from_str(&step_yaml)?;
///
/// let mut session = Session::new(serde_json::json!({"working_dir": "/tmp"}))?;
/// session.execute_step(step_wf, Duration::from_secs(60)).await?;
///
/// let wf = session.build("my-pipeline", "default", "0.1.0");
/// println!("{}", serde_yaml::to_string(&wf)?);
/// # Ok(())
/// # }
/// ```
pub struct Session {
    engine: Arc<DurableEngine>,
    ctx: serde_json::Value,
    steps: Vec<(String, TaskDefinition)>,
}

impl Session {
    /// Create a new session with the given initial context.
    pub fn new(ctx: serde_json::Value) -> Result<Self> {
        let engine = DurableEngineBuilder::new()
            .build()
            .map_err(|e| Error::Engine {
                message: e.to_string(),
            })?;
        Ok(Self {
            engine: Arc::new(engine),
            ctx,
            steps: Vec::new(),
        })
    }

    /// The current accumulated context.
    pub fn context(&self) -> &serde_json::Value {
        &self.ctx
    }

    /// Replace the current context.  Use this to inject a modified context
    /// between steps (e.g. narrowing a list before a download step).
    pub fn set_context(&mut self, ctx: serde_json::Value) {
        self.ctx = ctx;
    }

    /// Execute one workflow step asynchronously.
    ///
    /// All tasks in `workflow.do_` are run sequentially, their definitions are
    /// appended to the session's accumulated step list, and the output becomes
    /// the new context.
    pub async fn execute_step(
        &mut self,
        workflow: WorkflowDefinition,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        // Collect the task definitions before moving `workflow` into the engine.
        let new_steps: Vec<(String, TaskDefinition)> = workflow
            .do_
            .entries
            .iter()
            .flat_map(|entry| entry.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect();

        let handle = Arc::clone(&self.engine)
            .execute(workflow, self.ctx.clone())
            .await
            .map_err(|e| Error::Engine {
                message: e.to_string(),
            })?;

        let result = handle
            .wait_for_completion(timeout)
            .await
            .map_err(|e| Error::Engine {
                message: e.to_string(),
            })?;

        self.ctx = result.clone();
        self.steps.extend(new_steps);
        Ok(result)
    }

    /// Accumulate the tasks from `workflow.do_` without executing them.
    ///
    /// Used in script mode where `__main__` will run the assembled workflow
    /// through the full engine rather than executing steps one at a time.
    pub fn add_step(&mut self, workflow: WorkflowDefinition) {
        let new_steps: Vec<(String, TaskDefinition)> = workflow
            .do_
            .entries
            .iter()
            .flat_map(|entry| entry.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect();
        self.steps.extend(new_steps);
    }

    /// Synchronous wrapper around [`execute_step`](Self::execute_step).
    ///
    /// Creates a temporary Tokio runtime for the duration of the call, safe
    /// to call from a notebook cell or any non-async context.
    pub fn execute_step_sync(
        &mut self,
        workflow: WorkflowDefinition,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let new_steps: Vec<(String, TaskDefinition)> = workflow
            .do_
            .entries
            .iter()
            .flat_map(|entry| entry.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect();

        let engine = Arc::clone(&self.engine);
        let ctx_snapshot = self.ctx.clone();

        let rt = tokio::runtime::Runtime::new().map_err(|e| Error::Runtime {
            message: format!("Failed to create Tokio runtime: {e}"),
        })?;

        let result = rt.block_on(async move {
            let handle = engine
                .execute(workflow, ctx_snapshot)
                .await
                .map_err(|e| Error::Engine {
                    message: e.to_string(),
                })?;
            handle
                .wait_for_completion(timeout)
                .await
                .map_err(|e| Error::Engine {
                    message: e.to_string(),
                })
        })?;

        self.ctx = result.clone();
        self.steps.extend(new_steps);
        Ok(result)
    }

    /// Build a [`WorkflowDefinition`] from all accumulated steps.
    pub fn build(&self, name: &str, namespace: &str, version: &str) -> WorkflowDefinition {
        let mut do_ = Map::new();
        for (step_name, task) in &self.steps {
            do_.add(step_name.clone(), task.clone());
        }

        let mut wf = WorkflowDefinition::new(WorkflowDefinitionMetadata::new(
            namespace,
            name,
            version,
            None,
            None,
            None,
        ));
        wf.do_ = do_;
        wf
    }

    /// Serialise all accumulated steps as a YAML workflow string.
    pub fn export_yaml(&self, name: &str, namespace: &str, version: &str) -> Result<String> {
        let wf = self.build(name, namespace, version);
        serde_yaml::to_string(&wf).map_err(|e| Error::Yaml {
            message: format!("Failed to serialise workflow: {e}"),
        })
    }
}
