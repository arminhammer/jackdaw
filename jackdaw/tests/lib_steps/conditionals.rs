//! tasks/conditionals.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Order {
    pub cents: u64,
    #[serde(default)]
    pub express: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Decision {
    pub approved: bool,
}

pub struct Branches(pub Arc<Probe>);

#[tasks]
impl Branches {
    #[task(name = "cond.score")]
    pub async fn score(self: Arc<Self>, ctx: TaskContext, order: Order) -> Result<Risk, TaskError> {
        self.0.typed("score", &ctx, &order).await
    }
    #[task(name = "cond.auto_approve")]
    pub async fn auto_approve(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Order,
    ) -> Result<Decision, TaskError> {
        self.0.typed("auto_approve", &ctx, &order).await
    }
    #[task(name = "cond.manual_review")]
    pub async fn manual_review(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Order,
    ) -> Result<Decision, TaskError> {
        self.0.typed("manual_review", &ctx, &order).await
    }
    #[task(name = "cond.ship")]
    pub async fn ship(self: Arc<Self>, ctx: TaskContext, order: Order) -> Result<(), TaskError> {
        self.0.typed("ship", &ctx, &order).await
    }
    #[task(name = "cond.courier")]
    pub async fn courier(self: Arc<Self>, ctx: TaskContext, order: Order) -> Result<(), TaskError> {
        self.0.typed("courier", &ctx, &order).await
    }
    #[task(name = "cond.post")]
    pub async fn post(self: Arc<Self>, ctx: TaskContext, order: Order) -> Result<(), TaskError> {
        self.0.typed("post", &ctx, &order).await
    }
    #[task(name = "cond.fraud_check")]
    pub async fn fraud_check(
        self: Arc<Self>,
        ctx: TaskContext,
        order: Order,
    ) -> Result<(), TaskError> {
        self.0.typed("fraud_check", &ctx, &order).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("score", |_| json!("Low"));
    probe.default_output("auto_approve", |_| json!({"approved": true}));
    probe.default_output("manual_review", |_| json!({"approved": true}));
    for name in ["ship", "courier", "post", "fraud_check"] {
        probe.default_output(name, |_| json!(null));
    }
}

#[workflow(namespace = "cond", name = "router", version = "1")]
pub struct Router;

#[workflow_methods]
impl Router {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Decision> {
        let risk = ctx.call(Branches::score, &order).await?;
        let decision = match risk {
            Risk::Low => ctx.call(Branches::auto_approve, &order).await?,
            Risk::Medium if order.cents < 10_000 => {
                ctx.call(Branches::auto_approve, &order).await?
            }
            Risk::Medium | Risk::High => ctx.call(Branches::manual_review, &order).await?,
        };
        if decision.approved {
            ctx.call(Branches::ship, &order).await?;
        }
        Ok(decision)
    }
}

/// The same identity with a stricter guard: incompatible with histories that took the old branch.
pub mod stricter {
    use super::*;

    #[workflow(namespace = "cond", name = "router", version = "1")]
    pub struct Router;

    #[workflow_methods]
    impl Router {
        #[run]
        async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Decision> {
            let risk = ctx.call(Branches::score, &order).await?;
            let decision = match risk {
                Risk::Low => ctx.call(Branches::auto_approve, &order).await?,
                Risk::Medium if order.cents < 100 => {
                    ctx.call(Branches::auto_approve, &order).await?
                }
                Risk::Medium | Risk::High => ctx.call(Branches::manual_review, &order).await?,
            };
            if decision.approved {
                ctx.call(Branches::ship, &order).await?;
            }
            Ok(decision)
        }
    }
}

/// The same identity with a fraud check added behind a patch.
pub mod patched {
    use super::*;

    #[workflow(namespace = "cond", name = "router", version = "1")]
    pub struct Router;

    #[workflow_methods]
    impl Router {
        #[run]
        async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Decision> {
            if ctx.patched("fraud-check") {
                ctx.call(Branches::fraud_check, &order).await?;
            }
            let risk = ctx.call(Branches::score, &order).await?;
            let decision = match risk {
                Risk::Low => ctx.call(Branches::auto_approve, &order).await?,
                Risk::Medium if order.cents < 10_000 => {
                    ctx.call(Branches::auto_approve, &order).await?
                }
                Risk::Medium | Risk::High => ctx.call(Branches::manual_review, &order).await?,
            };
            if decision.approved {
                ctx.call(Branches::ship, &order).await?;
            }
            Ok(decision)
        }
    }
}

#[workflow(namespace = "cond", name = "shipper", version = "1")]
pub struct Shipper;

#[workflow_methods]
impl Shipper {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<()> {
        if input.express {
            ctx.call(Branches::courier, &input).await?;
        } else {
            ctx.call(Branches::post, &input).await?;
        }
        Ok(())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Router>("cond.router").input(json!({"cents": 500})),
        Fixture::of::<Shipper>("cond.shipper").input(json!({"cents": 500})),
    ]
}

pub fn variants() -> Vec<Fixture> {
    vec![
        Fixture::of::<stricter::Router>("cond.router.stricter"),
        Fixture::of::<patched::Router>("cond.router.patched"),
    ]
}

/// Engine task names carry a prefix; scenarios use the short name.
fn ran_tasks(world: &LibWorld) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for call in world.probe.calls() {
        if names.last() != Some(&call.name) {
            names.push(call.name);
        }
    }
    names
}

#[given(regex = r#"^a workflow "Router" whose run does:$"#)]
fn router(world: &mut LibWorld) {
    world.set_fixture(super::fixture("cond.router"));
}

#[given(regex = r#"^"Score" returns "(\w+)" for an order of (\d+) cents$"#)]
fn score_returns(world: &mut LibWorld, risk: String, cents: u64) {
    world
        .probe
        .script("score", |s| s.output = Some(json!(risk)));
    world.input = Some(json!({ "cents": cents }));
}

#[given(regex = r#"^"ManualReview" approves$"#)]
fn manual_approves(world: &mut LibWorld) {
    world.probe.script("manual_review", |s| {
        s.output = Some(json!({"approved": true}))
    });
}

#[given(regex = r#"^"Score" returns "(\w+)" and "ManualReview" rejects$"#)]
fn score_and_reject(world: &mut LibWorld, risk: String) {
    world
        .probe
        .script("score", |s| s.output = Some(json!(risk)));
    world.probe.script("manual_review", |s| {
        s.output = Some(json!({"approved": false}))
    });
}

#[when(regex = r#"^I execute "Router"$"#)]
async fn execute_router(world: &mut LibWorld) {
    let input = world.input.clone().unwrap_or_else(|| json!({"cents": 500}));
    world
        .start_fixture("it", super::fixture("cond.router"), input)
        .await;
}

#[then(regex = r"^the tasks that ran are (.+)$")]
async fn tasks_that_ran(world: &mut LibWorld, list: String) {
    world.settle().await;
    let expected: Vec<String> = list
        .split(',')
        .map(|name| name.trim().to_string())
        .collect();
    assert_eq!(ran_tasks(world), expected);
}

#[then(regex = r#"^"(\w+)" never ran$"#)]
async fn never_ran(world: &mut LibWorld, task: String) {
    world.settle().await;
    assert_eq!(world.probe.count(&task), 0, "{task} ran");
}

#[then(regex = r#"^the journal has no step for "(\w+)"$"#)]
async fn no_step_for(world: &mut LibWorld, task: String) {
    let last = world.last();
    let id = world.runs[last].id.clone();
    let history = world.engine().history(&id).await.expect("history");
    assert!(!history.iter().any(|event| matches!(event,
        Event::TaskStarted { task_name, .. } | Event::TaskCompleted { task_name, .. }
            if task_name.ends_with(&format!(".{task}")))));
}

#[given(regex = r#"^a completed "Router" instance where "Score" returned "(\w+)"$"#)]
async fn completed_router(world: &mut LibWorld, risk: String) {
    world
        .probe
        .script("score", |s| s.output = Some(json!(risk)));
    world
        .start_fixture("it", super::fixture("cond.router"), json!({"cents": 500}))
        .await;
    let last = world.last();
    world.outcome(last).await.value();
}

#[when(regex = r#"^"Score" would now return "(\w+)"$"#)]
fn score_now(world: &mut LibWorld, risk: String) {
    world
        .probe
        .script("score", |s| s.output = Some(json!(risk)));
    world.mark = Some(world.probe.calls().len());
}

#[then(regex = r#"^the replay takes the "ManualReview" branch$"#)]
fn replay_manual(world: &mut LibWorld) {
    // A replay that took another branch would diverge at the second step.
    assert!(world.divergences.as_ref().expect("replayed").is_empty());
}

#[then(regex = r#"^"score" is not called during the replay$"#)]
fn score_not_called(world: &mut LibWorld) {
    assert_eq!(Some(world.probe.calls().len()), world.mark);
}

#[given(
    regex = r#"^"Score" returned "(\w+)" and the engine is dropped while "ManualReview" runs$"#
)]
async fn dropped_in_review(world: &mut LibWorld, risk: String) {
    world
        .probe
        .script("score", |s| s.output = Some(json!(risk)));
    world.probe.script("manual_review", |s| s.block = true);
    world
        .start_fixture("it", super::fixture("cond.router"), json!({"cents": 500}))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("manual review runs", || probe.running("manual_review"))
        .await;
    world.crash().await;
    world.probe.release("manual_review");
}

#[then(regex = r#"^"ManualReview" runs to completion$"#)]
async fn review_completes(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    assert_eq!(world.probe.completed("manual_review"), 1);
}

#[then(regex = r#"^"AutoApprove" never runs$"#)]
fn auto_never(world: &mut LibWorld) {
    assert_eq!(world.probe.count("auto_approve"), 0);
}

#[given(regex = r#"^a workflow that does "if input\.express .+"$"#)]
fn express_workflow(world: &mut LibWorld) {
    world.set_fixture(super::fixture("cond.shipper"));
}

#[then(regex = r#"^only "(\w+)" ran$"#)]
async fn only_ran(world: &mut LibWorld, task: String) {
    world.settle().await;
    assert_eq!(ran_tasks(world), vec![task]);
}

#[given(regex = r#"^completed instances that took the "AutoApprove" branch for "Medium" risk$"#)]
async fn completed_medium(world: &mut LibWorld) {
    world
        .probe
        .script("score", |s| s.output = Some(json!("Medium")));
    for cents in [500, 900] {
        world
            .start_fixture("it", super::fixture("cond.router"), json!({"cents": cents}))
            .await;
    }
    world.settle().await;
}

#[when(regex = r#"^the code changes the guard to "order\.cents < 100"$"#)]
async fn code_changes_guard(world: &mut LibWorld) {
    world.crash().await;
    world.only = Some(vec![super::variant("cond.router.stricter").register]);
}

#[then(regex = r"^each affected instance is reported as a divergence at the branch's first step$")]
fn divergence_at_branch(world: &mut LibWorld) {
    let divergences = world.divergences.clone().expect("replayed");
    assert_eq!(divergences.len(), 2, "{divergences:?}");
    for divergence in divergences {
        assert_eq!(divergence.step, 1);
        assert!(
            divergence.expected.contains("manual_review"),
            "{divergence:?}"
        );
    }
}

#[given(regex = r#"^a running instance started before the code added "if ctx\.patched.+"$"#)]
async fn running_before_patch(world: &mut LibWorld) {
    world
        .probe
        .script("score", |s| s.output = Some(json!("High")));
    world.probe.script("manual_review", |s| s.block = true);
    world
        .start_fixture("old", super::fixture("cond.router"), json!({"cents": 500}))
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("manual review runs", || probe.running("manual_review"))
        .await;
}

#[when("it resumes on the new code")]
async fn resumes_on_new_code(world: &mut LibWorld) {
    world.crash().await;
    let patched = super::variant("cond.router.patched");
    world.only = Some(vec![patched.register.clone()]);
    world.probe.release("manual_review");
    let index = world.run_index("old");
    world.runs[index].fixture = Some(patched);
    world.resume(index).await;
    world.outcome(index).await.value();
}

#[then(regex = r#"^"fraud_check" does not run for it$"#)]
fn fraud_not_for_old(world: &mut LibWorld) {
    let old = world.runs[world.run_index("old")].id.clone();
    assert!(
        !world
            .probe
            .calls_of("fraud_check")
            .iter()
            .any(|c| c.instance == old)
    );
}

#[then(regex = r#"^a new instance runs "fraud_check"$"#)]
async fn new_runs_fraud(world: &mut LibWorld) {
    let patched = super::variant("cond.router.patched");
    world
        .start_fixture("new", patched, json!({"cents": 500}))
        .await;
    let index = world.run_index("new");
    world.outcome(index).await.value();
    let id = world.runs[index].id.clone();
    assert!(
        world
            .probe
            .calls_of("fraud_check")
            .iter()
            .any(|c| c.instance == id)
    );
    let _: Value = json!(null);
}
