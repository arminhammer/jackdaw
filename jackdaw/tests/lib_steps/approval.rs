//! workflows/approval_escalation.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Expense {
    pub cents: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Decision {
    Approved,
    Rejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ApprovalOutcome {
    AutoApproved,
    Approved,
    Rejected,
    Expired,
}

pub struct Approvals(pub Arc<Probe>);

#[tasks]
impl Approvals {
    #[task(name = "approval.chain")]
    pub async fn approver_chain(
        self: Arc<Self>,
        ctx: TaskContext,
        expense: Expense,
    ) -> Result<Vec<String>, TaskError> {
        self.0.typed("approver_chain", &ctx, &expense).await
    }
    #[task(name = "approval.request")]
    pub async fn request_approval(
        self: Arc<Self>,
        ctx: TaskContext,
        request: (String, Expense),
    ) -> Result<(), TaskError> {
        self.0.typed("request_approval", &ctx, &request).await
    }
    #[task(name = "approval.reimburse")]
    pub async fn reimburse(
        self: Arc<Self>,
        ctx: TaskContext,
        expense: Expense,
    ) -> Result<(), TaskError> {
        self.0.typed("reimburse", &ctx, &expense).await
    }
    #[task(name = "approval.notify_rejected")]
    pub async fn notify_rejected(
        self: Arc<Self>,
        ctx: TaskContext,
        reason: String,
    ) -> Result<(), TaskError> {
        self.0.typed("notify_rejected", &ctx, &reason).await
    }
    #[task(name = "approval.notify_expired")]
    pub async fn notify_expired(
        self: Arc<Self>,
        ctx: TaskContext,
        expense: Expense,
    ) -> Result<(), TaskError> {
        self.0.typed("notify_expired", &ctx, &expense).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("approver_chain", |_| json!(["manager", "director", "cfo"]));
    for name in [
        "request_approval",
        "reimburse",
        "notify_rejected",
        "notify_expired",
    ] {
        probe.default_output(name, |_| json!(null));
    }
}

#[workflow(namespace = "finance", name = "expense-approval", version = "1.0.0")]
pub struct ExpenseApproval {
    decision: Option<Decision>,
    approver: Option<String>,
}

#[workflow_methods]
impl ExpenseApproval {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self {
            decision: None,
            approver: None,
        }
    }

    #[run]
    async fn run(
        ctx: &mut WorkflowContext<Self>,
        expense: Expense,
    ) -> WorkflowResult<ApprovalOutcome> {
        if expense.cents <= 10_000 {
            return Ok(ApprovalOutcome::AutoApproved);
        }
        let chain = ctx.call(Approvals::approver_chain, &expense).await?;
        for approver in chain {
            ctx.call(
                Approvals::request_approval,
                &(approver.clone(), expense.clone()),
            )
            .await?;
            let decided = jackdaw::select! {
                _ = ctx.wait_condition(|s| s.decision.is_some()) => true,
                _ = ctx.sleep(48.hours()) => false,
            };
            if decided {
                break;
            }
        }
        match ctx.state(|s| s.decision.clone()) {
            Some(Decision::Approved) => {
                ctx.call(Approvals::reimburse, &expense).await?;
                Ok(ApprovalOutcome::Approved)
            }
            Some(Decision::Rejected(reason)) => {
                ctx.call(Approvals::notify_rejected, &reason).await?;
                Ok(ApprovalOutcome::Rejected)
            }
            None => {
                ctx.call(Approvals::notify_expired, &expense).await?;
                Ok(ApprovalOutcome::Expired)
            }
        }
    }

    #[signal]
    fn decide(&mut self, _ctx: &mut SyncWorkflowContext<Self>, input: (String, Decision)) {
        self.approver = Some(input.0);
        self.decision = Some(input.1);
    }

    #[query]
    fn status(&self, _ctx: &WorkflowContextView) -> (Option<Decision>, Option<String>) {
        (self.decision.clone(), self.approver.clone())
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![Fixture::of::<ExpenseApproval>("approval.expense").input(json!({"cents": 50_000}))]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![("struct ExpenseApproval", "approval.expense")]
}

/// `("manager", Approved)` or `("director", Rejected("no receipt"))`.
fn decision(text: &str) -> (String, Decision) {
    let inner = text.trim().trim_start_matches('(').trim_end_matches(')');
    let (who, what) = inner.split_once(',').expect("(who, decision)");
    let who = who.trim().trim_matches('"').to_string();
    let what = what.trim();
    let decision = if what == "Approved" {
        Decision::Approved
    } else {
        let reason = what
            .trim_start_matches("Rejected(")
            .trim_end_matches(')')
            .trim_matches('"')
            .to_string();
        Decision::Rejected(reason)
    };
    (who, decision)
}

#[when(regex = r#"^I execute "ExpenseApproval" for (\d+) cents$"#)]
async fn execute_expense(world: &mut LibWorld, cents: u64) {
    world
        .start_fixture(
            "it",
            super::fixture("approval.expense"),
            json!({ "cents": cents }),
        )
        .await;
    world.quiesce().await;
}

async fn send_decision(world: &mut LibWorld, text: &str) {
    let handle = world.handle(world.last());
    let result = handle.signal(ExpenseApproval::decide, decision(text)).await;
    if let Err(error) = result {
        world
            .notes
            .insert("signal_error".into(), json!(error.to_string()));
    }
}

#[when(regex = r#"^after (\d+ \w+) I signal "decide" with (\(.+\))$"#)]
async fn after_decide(world: &mut LibWorld, span: String, text: String) {
    world.pass(duration(&span)).await;
    send_decision(world, &text).await;
}

#[when(regex = r#"^I signal "decide" with (\(.+\))$"#)]
async fn signal_decide(world: &mut LibWorld, text: String) {
    send_decision(world, &text).await;
}

#[when(regex = r"^(\d+ \w+) pass without a decision$")]
async fn pass_without_decision(world: &mut LibWorld, span: String) {
    world.pass(duration(&span)).await;
}

#[when(regex = r"^(\d+) hours pass$")]
async fn hours_pass(world: &mut LibWorld, hours: u64) {
    world.pass(Duration::from_secs(hours * 3600)).await;
}

#[when(regex = r"^(\d+) more hours pass$")]
async fn more_hours_pass(world: &mut LibWorld, hours: u64) {
    world.pass(Duration::from_secs(hours * 3600)).await;
}

#[then(regex = r#"^"RequestApproval" has run for "(\w+)"$"#)]
async fn requested_from(world: &mut LibWorld, who: String) {
    world.quiesce().await;
    let asked: Vec<Value> = world
        .probe
        .calls_of("request_approval")
        .into_iter()
        .map(|c| c.input[0].clone())
        .collect();
    assert!(asked.contains(&json!(who)), "{asked:?}");
}

#[then(regex = r#"^querying "status" returns no decision and no approver$"#)]
async fn status_empty(world: &mut LibWorld) {
    let handle = world.handle(world.last());
    let status = handle.query(ExpenseApproval::status).await.expect("query");
    assert_eq!(status, (None, None));
}

#[then(regex = r#"^"NotifyRejected" ran with "(.+)"$"#)]
async fn rejected_with(world: &mut LibWorld, reason: String) {
    world.settle().await;
    assert_eq!(
        world.probe.calls_of("notify_rejected")[0].input,
        json!(reason)
    );
}

#[when("the engine is dropped before the workflow reacts")]
async fn dropped_before_reacting(world: &mut LibWorld) {
    world.crash().await;
}

#[given("the manager approved and the workflow completed")]
async fn manager_approved(world: &mut LibWorld) {
    world
        .start_fixture(
            "it",
            super::fixture("approval.expense"),
            json!({ "cents": 50_000 }),
        )
        .await;
    world.quiesce().await;
    send_decision(world, "(\"manager\", Approved)").await;
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("Approved"));
}

#[then("the signal call fails because the instance is not running")]
fn signal_fails(world: &mut LibWorld) {
    let error = world
        .notes
        .get("signal_error")
        .and_then(Value::as_str)
        .expect("the signal failed");
    assert!(error.contains("not running"), "{error}");
}
