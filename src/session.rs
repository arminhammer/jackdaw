//! Interactive pipeline session for notebook-driven workflow development.
//!
//! A [`Session`] holds the evolving workflow context and an ordered list of
//! committed steps. The two core verbs are:
//!
//! - [`Session::preview`] — execute a step against the current context
//!   without recording anything. Safe to call repeatedly while iterating on
//!   a step definition.
//! - [`Session::commit`] — accept a step: execute it, record its definition
//!   together with before/after context snapshots, and advance the context.
//!   Committing a name that already exists *replaces* that step: the context
//!   is rewound to the snapshot taken before the original run, the new
//!   definition is executed, and every later step is marked stale.
//!
//! Stale steps are healed with [`Session::replay`], which re-executes the
//! recorded suffix in order. When the pipeline looks right, export it with
//! [`Session::build`] or [`Session::export_yaml`].
//!
//! Rewinding restores the *context* snapshot only — filesystem or other
//! external side effects of previously executed steps are not undone.

use crate::{builder::DurableEngineBuilder, durableengine::DurableEngine};
use serverless_workflow_core::models::{
    input::InputDataModelDefinition,
    map::Map,
    output::OutputDataModelDefinition,
    schema::SchemaDefinition,
    task::TaskDefinition,
    workflow::{WorkflowDefinition, WorkflowDefinitionMetadata},
};
use snafu::Snafu;
use std::{sync::Arc, time::Duration};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Engine error: {message}"))]
    Engine { message: String },

    #[snafu(display("YAML error: {message}"))]
    Yaml { message: String },

    #[snafu(display("Invalid step: {message}"))]
    InvalidStep { message: String },

    #[snafu(display("Unknown step: {name}"))]
    UnknownStep { name: String },

    #[snafu(display("Schema validation error: {message}"))]
    Schema { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// One committed step: its definition plus the context snapshots taken
/// around its most recent execution.
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub name: String,
    pub task: TaskDefinition,
    pub ctx_before: serde_json::Value,
    pub ctx_after: serde_json::Value,
    /// True when an earlier step was re-committed after this step ran, so
    /// this step's snapshots no longer reflect the current chain.
    pub stale: bool,
}

/// Summary of a committed step, as returned by [`Session::status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepStatus {
    pub name: String,
    pub stale: bool,
}

/// Interactive pipeline session.
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
/// let mut session = Session::new(serde_json::json!({"working_dir": "/tmp"}), None, None)?;
///
/// // Iterate freely: preview never mutates the session.
/// let candidate = session.preview(step_wf.clone(), Duration::from_secs(60)).await?;
/// println!("{candidate}");
///
/// // Happy with it? Commit to record the step and advance the context.
/// session.commit(step_wf, Duration::from_secs(60)).await?;
///
/// let wf = session.build("my-pipeline", "default", "0.1.0");
/// println!("{}", serde_yaml::to_string(&wf)?);
/// # Ok(())
/// # }
/// ```
pub struct Session {
    engine: Arc<DurableEngine>,
    ctx: serde_json::Value,
    steps: Vec<StepRecord>,
    /// JSON Schema for the workflow's input, derived from the typed
    /// signature at the language-binding layer. Validated against the seed
    /// context at creation and embedded in the exported workflow, where the
    /// engine enforces it on every execution.
    input_schema: Option<serde_json::Value>,
    /// JSON Schema for the workflow's output, embedded in the exported
    /// workflow and enforced by the engine on the final transformed output.
    output_schema: Option<serde_json::Value>,
}

impl Session {
    /// Create a new session with the given initial context.
    ///
    /// When `input_schema` is provided, the seed context is validated against
    /// it immediately — a session cannot be created from input that violates
    /// its own declared contract.
    pub fn new(
        ctx: serde_json::Value,
        input_schema: Option<serde_json::Value>,
        output_schema: Option<serde_json::Value>,
    ) -> Result<Self> {
        if let Some(schema) = input_schema.as_ref() {
            crate::durableengine::schema::check_document(schema, &ctx, "input")
                .map_err(|message| Error::Schema { message })?;
        }

        let engine = DurableEngineBuilder::new()
            .build()
            .map_err(|e| Error::Engine {
                message: e.to_string(),
            })?;
        Ok(Self {
            engine: Arc::new(engine),
            ctx,
            steps: Vec::new(),
            input_schema,
            output_schema,
        })
    }

    /// The current accumulated context.
    #[must_use]
    pub fn context(&self) -> &serde_json::Value {
        &self.ctx
    }

    /// Replace the current context.
    ///
    /// Escape hatch for injecting an arbitrary context. Step snapshots are
    /// not adjusted; prefer [`update_context`](Self::update_context) for
    /// targeted edits between steps.
    pub fn set_context(&mut self, ctx: serde_json::Value) {
        self.ctx = ctx;
    }

    /// Merge the top-level keys of `patch` into the current context
    /// (e.g. narrowing a list before committing the next step).
    pub fn update_context(&mut self, patch: serde_json::Value) {
        if let (Some(ctx), Some(patch)) = (self.ctx.as_object_mut(), patch.as_object()) {
            for (k, v) in patch {
                ctx.insert(k.clone(), v.clone());
            }
        }
    }

    /// The committed steps in order.
    #[must_use]
    pub fn steps(&self) -> &[StepRecord] {
        &self.steps
    }

    /// Name and staleness of every committed step, in order.
    #[must_use]
    pub fn status(&self) -> Vec<StepStatus> {
        self.steps
            .iter()
            .map(|s| StepStatus {
                name: s.name.clone(),
                stale: s.stale,
            })
            .collect()
    }

    /// Execute `workflow` against a copy of the current context and return
    /// the result. Neither the context nor the committed steps change, so
    /// previewing is idempotent — call it as often as you like while
    /// iterating on a step.
    pub async fn preview(
        &self,
        workflow: WorkflowDefinition,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        self.execute(workflow, self.ctx.clone(), timeout).await
    }

    /// Commit one step: execute it and record it with context snapshots.
    ///
    /// `workflow` must contain exactly one named top-level task (wrap
    /// multi-task blocks in a `do` task).
    ///
    /// If a step with the same name was committed before, the context is
    /// rewound to that step's `ctx_before`, the new definition replaces the
    /// old one in place, and every later step is marked stale — re-running a
    /// commit cell in a notebook is therefore the edit operation, not a
    /// duplicate append. Heal stale steps with [`replay`](Self::replay).
    ///
    /// Returns the new context.
    pub async fn commit(
        &mut self,
        workflow: WorkflowDefinition,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let (name, task) = single_task(&workflow)?;

        let position = self.steps.iter().position(|s| s.name == name);
        let ctx_before = position
            .and_then(|k| self.steps.get(k))
            .map_or_else(|| self.ctx.clone(), |s| s.ctx_before.clone());

        let ctx_after = self.execute(workflow, ctx_before.clone(), timeout).await?;

        let record = StepRecord {
            name,
            task,
            ctx_before,
            ctx_after: ctx_after.clone(),
            stale: false,
        };

        match position {
            Some(k) => {
                if let Some(slot) = self.steps.get_mut(k) {
                    *slot = record;
                }
                for later in self.steps.iter_mut().skip(k + 1) {
                    later.stale = true;
                }
            }
            None => self.steps.push(record),
        }

        self.ctx = ctx_after.clone();
        Ok(ctx_after)
    }

    /// Re-execute every step from the first stale one onward, in committed
    /// order, refreshing snapshots and the context as it goes. Steps after
    /// the first stale one are re-executed even if not themselves stale, so
    /// the snapshot chain stays consistent.
    ///
    /// `timeout` applies per step. Returns the final context; a no-op that
    /// returns the current context when nothing is stale.
    pub async fn replay(&mut self, timeout: Duration) -> Result<serde_json::Value> {
        let Some(first_stale) = self.steps.iter().position(|s| s.stale) else {
            return Ok(self.ctx.clone());
        };

        // Re-anchor: the stale suffix replays from the context produced by
        // the last fresh step before it, not from whatever self.ctx holds.
        let mut ctx = match first_stale.checked_sub(1).and_then(|i| self.steps.get(i)) {
            Some(prev) => prev.ctx_after.clone(),
            None => self
                .steps
                .get(first_stale)
                .map_or_else(|| self.ctx.clone(), |s| s.ctx_before.clone()),
        };

        for i in first_stale..self.steps.len() {
            let Some((name, task)) = self.steps.get(i).map(|s| (s.name.clone(), s.task.clone()))
            else {
                break;
            };
            let workflow = wrap_task(&name, task);
            let ctx_after = self.execute(workflow, ctx.clone(), timeout).await?;
            if let Some(step) = self.steps.get_mut(i) {
                step.ctx_before = ctx;
                step.ctx_after = ctx_after.clone();
                step.stale = false;
            }
            ctx = ctx_after;
        }

        self.ctx = ctx.clone();
        Ok(ctx)
    }

    /// Rewind the context to just before `name` ran and drop that step and
    /// everything after it. Returns the restored context.
    pub fn rollback(&mut self, name: &str) -> Result<serde_json::Value> {
        let (k, ctx_before) = self
            .steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.name == name)
            .map(|(k, s)| (k, s.ctx_before.clone()))
            .ok_or_else(|| Error::UnknownStep {
                name: name.to_string(),
            })?;
        self.ctx = ctx_before;
        self.steps.truncate(k);
        Ok(self.ctx.clone())
    }

    /// Build a [`WorkflowDefinition`] from all committed steps.
    ///
    /// The session's input and output schemas (when present) are embedded as
    /// the workflow's declared contracts, which the engine enforces on every
    /// execution of the exported workflow.
    #[must_use]
    pub fn build(&self, name: &str, namespace: &str, version: &str) -> WorkflowDefinition {
        let mut do_ = Map::new();
        for step in &self.steps {
            do_.add(step.name.clone(), step.task.clone());
        }

        let mut wf = WorkflowDefinition::new(WorkflowDefinitionMetadata::new(
            namespace, name, version, None, None, None,
        ));
        wf.do_ = do_;

        if let Some(doc) = self.input_schema.as_ref() {
            wf.input = Some(InputDataModelDefinition {
                schema: Some(inline_json_schema(doc.clone())),
                from: None,
            });
        }
        if let Some(doc) = self.output_schema.as_ref() {
            wf.output = Some(OutputDataModelDefinition {
                schema: Some(inline_json_schema(doc.clone())),
                as_: None,
            });
        }
        wf
    }

    /// Serialise all committed steps as a YAML workflow string.
    pub fn export_yaml(&self, name: &str, namespace: &str, version: &str) -> Result<String> {
        let wf = self.build(name, namespace, version);
        serde_yaml::to_string(&wf).map_err(|e| Error::Yaml {
            message: format!("Failed to serialise workflow: {e}"),
        })
    }

    async fn execute(
        &self,
        workflow: WorkflowDefinition,
        input: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let handle = self
            .engine
            .execute(workflow, input)
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
    }
}

/// Extract the single named top-level task from `workflow.do_`, erroring
/// when there are zero or several.
fn single_task(workflow: &WorkflowDefinition) -> Result<(String, TaskDefinition)> {
    let mut tasks = workflow
        .do_
        .entries
        .iter()
        .flat_map(|entry| entry.iter().map(|(k, v)| (k.clone(), v.clone())));

    let first = tasks.next().ok_or_else(|| Error::InvalidStep {
        message: "commit requires a workflow with exactly one task, got none".to_string(),
    })?;
    if tasks.next().is_some() {
        return Err(Error::InvalidStep {
            message: "commit requires a workflow with exactly one task; wrap multiple tasks in a 'do' task".to_string(),
        });
    }
    Ok(first)
}

/// Build an inline `format: json` schema definition from a schema document.
fn inline_json_schema(document: serde_json::Value) -> SchemaDefinition {
    SchemaDefinition {
        format: "json".to_string(),
        resource: None,
        document: Some(document),
    }
}

/// Wrap one task in a minimal single-step workflow for replay execution.
fn wrap_task(name: &str, task: TaskDefinition) -> WorkflowDefinition {
    let mut do_ = Map::new();
    do_.add(name.to_string(), task);
    let mut wf = WorkflowDefinition::new(WorkflowDefinitionMetadata::new(
        "session", "replay", "0.0.0", None, None, None,
    ));
    wf.do_ = do_;
    wf
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    /// Single-step workflow whose task merges `merge` (a JQ object literal,
    /// e.g. `{a: 1}`) into the context. The engine's `set` replaces the
    /// context with its output, so accumulation must be explicit.
    fn step(name: &str, merge: &str) -> WorkflowDefinition {
        let yaml = format!(
            r"
document:
  dsl: '1.0.2'
  namespace: test
  name: step
  version: '0.1.0'
do:
  - {name}:
      set: '${{ . + {merge} }}'
"
        );
        serde_yaml::from_str(&yaml).unwrap()
    }

    const TIMEOUT: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn preview_does_not_mutate() {
        let session = Session::new(json!({"seed": 1}), None, None).unwrap();

        let out = session.preview(step("a", "{a: 1}"), TIMEOUT).await.unwrap();
        assert_eq!(out.get("a"), Some(&json!(1)));

        // Neither context nor steps changed; previewing again gives the same result.
        assert_eq!(session.context(), &json!({"seed": 1}));
        assert!(session.status().is_empty());
        let again = session.preview(step("a", "{a: 1}"), TIMEOUT).await.unwrap();
        assert_eq!(again.get("a"), Some(&json!(1)));
    }

    #[tokio::test]
    async fn commit_appends_and_advances_context() {
        let mut session = Session::new(json!({}), None, None).unwrap();

        session.commit(step("a", "{a: 1}"), TIMEOUT).await.unwrap();
        let ctx = session.commit(step("b", "{b: 2}"), TIMEOUT).await.unwrap();

        assert_eq!(ctx.get("a"), Some(&json!(1)));
        assert_eq!(ctx.get("b"), Some(&json!(2)));
        assert_eq!(
            session.status(),
            vec![
                StepStatus {
                    name: "a".into(),
                    stale: false
                },
                StepStatus {
                    name: "b".into(),
                    stale: false
                },
            ]
        );
    }

    #[tokio::test]
    async fn commit_same_name_rewinds_and_marks_downstream_stale() {
        let mut session = Session::new(json!({}), None, None).unwrap();
        session.commit(step("a", "{a: 1}"), TIMEOUT).await.unwrap();
        session.commit(step("b", "{b: 2}"), TIMEOUT).await.unwrap();

        // Re-commit "a" with a new definition: rewound to before "a", so
        // "b"'s contribution is gone from the context until replay.
        let ctx = session.commit(step("a", "{a: 9}"), TIMEOUT).await.unwrap();
        assert_eq!(ctx.get("a"), Some(&json!(9)));
        assert_eq!(ctx.get("b"), None);
        assert_eq!(
            session.status(),
            vec![
                StepStatus {
                    name: "a".into(),
                    stale: false
                },
                StepStatus {
                    name: "b".into(),
                    stale: true
                },
            ]
        );

        // Replay heals the stale suffix.
        let ctx = session.replay(TIMEOUT).await.unwrap();
        assert_eq!(ctx.get("a"), Some(&json!(9)));
        assert_eq!(ctx.get("b"), Some(&json!(2)));
        assert!(session.status().iter().all(|s| !s.stale));
        assert_eq!(session.context(), &ctx);
    }

    #[tokio::test]
    async fn replay_without_stale_steps_is_a_no_op() {
        let mut session = Session::new(json!({}), None, None).unwrap();
        session.commit(step("a", "{a: 1}"), TIMEOUT).await.unwrap();

        let before = session.context().clone();
        let ctx = session.replay(TIMEOUT).await.unwrap();
        assert_eq!(ctx, before);
    }

    #[tokio::test]
    async fn rollback_truncates_and_restores_context() {
        let mut session = Session::new(json!({"seed": 1}), None, None).unwrap();
        session.commit(step("a", "{a: 1}"), TIMEOUT).await.unwrap();
        session.commit(step("b", "{b: 2}"), TIMEOUT).await.unwrap();

        let ctx = session.rollback("a").unwrap();
        assert_eq!(ctx, json!({"seed": 1}));
        assert!(session.status().is_empty());

        assert!(matches!(
            session.rollback("nope"),
            Err(Error::UnknownStep { .. })
        ));
    }

    #[tokio::test]
    async fn export_has_no_duplicates_after_recommit() {
        let mut session = Session::new(json!({}), None, None).unwrap();
        session.commit(step("a", "{a: 1}"), TIMEOUT).await.unwrap();
        session.commit(step("a", "{a: 2}"), TIMEOUT).await.unwrap();
        session.commit(step("b", "{b: 3}"), TIMEOUT).await.unwrap();

        let wf = session.build("pipeline", "test", "0.1.0");
        let names: Vec<String> = wf
            .do_
            .entries
            .iter()
            .flat_map(|e| e.keys().cloned())
            .collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);

        // The export carries the latest committed definition, not the replaced one.
        let yaml = session.export_yaml("pipeline", "test", "0.1.0").unwrap();
        assert!(yaml.contains("{a: 2}"));
        assert!(!yaml.contains("{a: 1}"));
    }

    #[tokio::test]
    async fn commit_rejects_multi_task_workflows() {
        let mut session = Session::new(json!({}), None, None).unwrap();
        let yaml = r"
document:
  dsl: '1.0.2'
  namespace: test
  name: step
  version: '0.1.0'
do:
  - a:
      set:
        a: 1
  - b:
      set:
        b: 2
";
        let wf: WorkflowDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(
            session.commit(wf, TIMEOUT).await,
            Err(Error::InvalidStep { .. })
        ));
    }

    #[tokio::test]
    async fn seed_context_is_validated_against_input_schema() {
        let schema = json!({
            "type": "object",
            "required": ["working_dir"],
            "properties": {"working_dir": {"type": "string"}}
        });

        assert!(Session::new(json!({"working_dir": "/tmp"}), Some(schema.clone()), None).is_ok());

        // .err() avoids requiring Session: Debug (unwrap_err would).
        let err = Session::new(json!({}), Some(schema), None).err().unwrap();
        assert!(matches!(err, Error::Schema { .. }));
        assert!(err.to_string().contains("working_dir"));
    }

    #[tokio::test]
    async fn export_embeds_input_and_output_schemas() {
        let input_schema = json!({"type": "object", "required": ["seed"]});
        let output_schema = json!({"type": "object", "required": ["a"]});
        let mut session = Session::new(
            json!({"seed": 1}),
            Some(input_schema.clone()),
            Some(output_schema.clone()),
        )
        .unwrap();
        session.commit(step("a", "{a: 1}"), TIMEOUT).await.unwrap();

        let wf = session.build("pipeline", "test", "0.1.0");
        let embedded_in = wf.input.unwrap().schema.unwrap();
        assert_eq!(embedded_in.format, "json");
        assert_eq!(embedded_in.document, Some(input_schema));
        let embedded_out = wf.output.unwrap().schema.unwrap();
        assert_eq!(embedded_out.document, Some(output_schema));
    }

    #[tokio::test]
    async fn update_context_merges_top_level_keys() {
        let mut session = Session::new(json!({"a": 1, "b": 2}), None, None).unwrap();
        session.update_context(json!({"b": 5, "c": 6}));
        assert_eq!(session.context(), &json!({"a": 1, "b": 5, "c": 6}));
    }
}
