//! workflows/subscription_billing.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Start {
    pub invoices_so_far: u32,
    pub plan: Plan,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Summary {
    pub invoices: u32,
}

pub struct Billing(pub Arc<Probe>);

#[tasks]
impl Billing {
    #[task(name = "billing.charge_card")]
    pub async fn charge_card(
        self: Arc<Self>,
        ctx: TaskContext,
        plan: Plan,
    ) -> Result<String, TaskError> {
        self.0.typed("charge_card", &ctx, &plan).await
    }
    #[task(name = "billing.send_invoice")]
    pub async fn send_invoice(
        self: Arc<Self>,
        ctx: TaskContext,
        invoice: String,
    ) -> Result<(), TaskError> {
        self.0.typed("send_invoice", &ctx, &invoice).await
    }
    #[task(name = "billing.dunning")]
    pub async fn dunning(self: Arc<Self>, ctx: TaskContext, plan: Plan) -> Result<(), TaskError> {
        self.0.typed("dunning", &ctx, &plan).await
    }
    #[task(name = "billing.apply_loyalty_credit")]
    pub async fn apply_loyalty_credit(
        self: Arc<Self>,
        ctx: TaskContext,
        plan: Plan,
    ) -> Result<(), TaskError> {
        self.0.typed("apply_loyalty_credit", &ctx, &plan).await
    }
    #[task(name = "billing.close_account")]
    pub async fn close_account(
        self: Arc<Self>,
        ctx: TaskContext,
        start: Start,
    ) -> Result<(), TaskError> {
        self.0.typed("close_account", &ctx, &start).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("charge_card", |plan| {
        json!(format!("inv-{}", plan["name"].as_str().unwrap_or("?")))
    });
    for name in [
        "send_invoice",
        "dunning",
        "apply_loyalty_credit",
        "close_account",
    ] {
        probe.default_output(name, |_| json!(null));
    }
}

macro_rules! subscription {
    ($module:ident, patch: $patch:expr, continue_every: $every:expr) => {
        pub mod $module {
            use super::*;

            #[workflow(namespace = "billing", name = "subscription", version = "1.0.0")]
            pub struct Subscription {
                pub plan: Option<Plan>,
                pub cancelled: bool,
            }

            #[workflow_methods]
            impl Subscription {
                #[init]
                fn new(_ctx: &WorkflowContextView) -> Self {
                    Self {
                        plan: None,
                        cancelled: false,
                    }
                }

                #[run]
                async fn run(
                    ctx: &mut WorkflowContext<Self>,
                    start: Start,
                ) -> WorkflowResult<Summary> {
                    let mut invoices = start.invoices_so_far;
                    loop {
                        let renew = jackdaw::select! {
                            _ = ctx.wait_condition(|s| s.cancelled) => false,
                            _ = ctx.sleep(30.days()) => true,
                        };
                        if !renew {
                            break;
                        }
                        let plan = ctx
                            .state(|s| s.plan.clone())
                            .unwrap_or_else(|| start.plan.clone());
                        match ctx
                            .call(Billing::charge_card, &plan)
                            .retry(Retry::exponential(4))
                            .await
                        {
                            Ok(invoice) => {
                                invoices += 1;
                                ctx.call(Billing::send_invoice, &invoice).await?;
                            }
                            Err(_) => {
                                ctx.call(Billing::dunning, &plan).await?;
                                break;
                            }
                        }
                        if $patch && ctx.patched("annual-discount") && invoices % 12 == 0 {
                            ctx.call(Billing::apply_loyalty_credit, &plan).await?;
                        }
                        let every: u32 = $every;
                        if every > 0 && invoices % every == 0 {
                            let plan = ctx
                                .state(|s| s.plan.clone())
                                .unwrap_or_else(|| start.plan.clone());
                            return ctx.continue_as_new(Start {
                                invoices_so_far: invoices,
                                plan,
                            });
                        }
                    }
                    ctx.call(Billing::close_account, &start).await?;
                    Ok(Summary { invoices })
                }

                #[signal]
                pub fn change_plan(&mut self, _ctx: &mut SyncWorkflowContext<Self>, plan: Plan) {
                    self.plan = Some(plan);
                }

                #[signal]
                pub fn cancel(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
                    self.cancelled = true;
                }

                #[query]
                pub fn plan(&self, _ctx: &WorkflowContextView) -> Option<Plan> {
                    self.plan.clone()
                }
            }
        }
    };
}

subscription!(current, patch: true, continue_every: 0);
subscription!(before_discount, patch: false, continue_every: 0);
subscription!(bounded, patch: true, continue_every: 24);

use current::Subscription;

pub fn fixtures() -> Vec<Fixture> {
    vec![Fixture::of::<Subscription>("billing.subscription").input(start("basic"))]
}

pub fn variants() -> Vec<Fixture> {
    vec![
        Fixture::of::<before_discount::Subscription>("billing.subscription.before-discount"),
        Fixture::of::<bounded::Subscription>("billing.subscription.bounded"),
    ]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![("struct Subscription {", "billing.subscription")]
}

fn start(plan: &str) -> serde_json::Value {
    json!({ "invoices_so_far": 0, "plan": { "name": plan } })
}

#[when(regex = r#"^I start a subscription(?: on "(\w+)")? and (\d+) (days|years) pass$"#)]
async fn start_and_pass(world: &mut LibWorld, plan: String, n: u64, unit: String) {
    let days = if unit == "years" { n * 365 } else { n };
    world.endless = true;
    let plan = if plan.is_empty() {
        "basic".to_string()
    } else {
        plan
    };
    let fixture = world
        .fixture
        .clone()
        .filter(|f| f.name.starts_with("billing."))
        .unwrap_or_else(|| super::fixture("billing.subscription"));
    world.start_fixture("it", fixture, start(&plan)).await;
    world.pass(Duration::from_secs(days * 86_400)).await;
}

#[when(regex = r#"^I signal "change_plan" with "(\w+)"$"#)]
async fn change_plan(world: &mut LibWorld, plan: String) {
    let handle = world.handle(world.last());
    handle
        .signal(Subscription::change_plan, Plan { name: plan })
        .await
        .expect("signal");
}

#[when(regex = r"^(\d+) more days pass$")]
async fn more_days(world: &mut LibWorld, days: u64) {
    world.pass(Duration::from_secs(days * 86_400)).await;
}

#[then(regex = r#"^the first charge was for "(\w+)" and the second for "(\w+)"$"#)]
fn charges_for(world: &mut LibWorld, first: String, second: String) {
    let plans: Vec<serde_json::Value> = world
        .probe
        .calls_of("charge_card")
        .into_iter()
        .map(|c| c.input["name"].clone())
        .collect();
    assert_eq!(plans[..2], [json!(first), json!(second)]);
}

#[when(regex = r#"^I signal "cancel"$"#)]
async fn cancel(world: &mut LibWorld) {
    let handle = world.handle(world.last());
    handle
        .signal(Subscription::cancel, ())
        .await
        .expect("signal");
}

#[then(regex = r"^the workflow completes with (\d+) invoices?$")]
async fn completes_with_invoices(world: &mut LibWorld, n: u32) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!({ "invoices": n }));
}

#[then(regex = r"^no further charges happen after (\d+) more days$")]
async fn no_further(world: &mut LibWorld, days: u64) {
    let before = world.probe.count("charge_card");
    world.pass(Duration::from_secs(days * 86_400)).await;
    assert_eq!(world.probe.count("charge_card"), before);
}

#[given(regex = r#"^"ChargeCard" fails every time$"#)]
fn card_fails(world: &mut LibWorld) {
    world.probe.script("charge_card", |s| {
        s.fault = Some(FaultSpec::retryable("card declined"))
    });
}

#[then(regex = r#"^"(\w+)" was attempted (\d+) times$"#)]
async fn attempted(world: &mut LibWorld, task: String, n: usize) {
    world.quiesce().await;
    assert_eq!(world.probe.count(&task), n);
}

#[given(regex = r#"^the loop ends each 24th cycle with "return ctx\.continue_as_new\(.+\)"$"#)]
fn bounded_variant(world: &mut LibWorld) {
    let bounded = super::variant("billing.subscription.bounded");
    world.only = Some(vec![bounded.register.clone()]);
    world.set_fixture(bounded);
}

#[then("the instance has continued as new at least once")]
async fn continued(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    assert!(details.run >= 2, "{details:?}");
}

#[then(regex = r"^the current run's journal holds no more than (\d+) billing cycles$")]
async fn bounded_journal(world: &mut LibWorld, cycles: usize) {
    let id = world.runs[world.last()].id.clone();
    let details = world.engine().describe(&id).await.expect("describe");
    // Each cycle records a sleep, a race and a charge, an invoice and a patch check at most.
    assert!(details.steps <= cycles * 5, "{details:?}");
}

#[then(regex = r#"^querying "plan" on the original instance id still works$"#)]
async fn query_plan(world: &mut LibWorld) {
    let id = world.runs[world.last()].id.clone();
    let handle = (super::variant("billing.subscription.bounded").attach)(world.engine(), id)
        .await
        .expect("attach");
    handle
        .query(bounded::Subscription::plan)
        .await
        .expect("query");
}

#[then(regex = r"^no charge happens until day (\d+)$")]
async fn none_until(world: &mut LibWorld, day: u64) {
    let started = world.runs[world.last()].started_virtual;
    let target = started + chrono::Duration::days(i64::try_from(day).unwrap())
        - chrono::Duration::seconds(1);
    let wait = (target - world.clock.now()).to_std().unwrap_or_default();
    world.pass(wait).await;
    assert_eq!(world.probe.count("charge_card"), 0);
}

#[then(regex = r"^exactly one charge happens on day (\d+)$")]
async fn one_on_day(world: &mut LibWorld, _day: u64) {
    world.pass(Duration::from_secs(1)).await;
    assert_eq!(world.probe.count("charge_card"), 1);
}

#[given(regex = r#"^a subscription started before "annual-discount" was added$"#)]
async fn before_discount(world: &mut LibWorld) {
    let old = super::variant("billing.subscription.before-discount");
    world.only = Some(vec![old.register.clone()]);
    world.start_fixture("old", old, start("basic")).await;
    world.pass(Duration::from_secs(65 * 86_400)).await;
}

#[when(regex = r"^it resumes on the new code and reaches its 12th invoice$")]
async fn resumes_new_code(world: &mut LibWorld) {
    world.crash().await;
    world.only = None;
    let index = world.run_index("old");
    world.runs[index].fixture = Some(super::fixture("billing.subscription"));
    world.resume(index).await;
    world.pass(Duration::from_secs(300 * 86_400)).await;
    assert!(world.probe.count("charge_card") >= 12);
}

#[then(regex = r#"^"ApplyLoyaltyCredit" does not run for it$"#)]
fn no_loyalty(world: &mut LibWorld) {
    assert_eq!(world.probe.count("apply_loyalty_credit"), 0);
}
