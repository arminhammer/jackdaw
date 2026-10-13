//! Workflows written in Rust. Control flow is ordinary Rust (`if`, `match`, loops, `?`); every effect is a typed task.
//!
//! All tests use `TestEngine`, which runs on tokio's paused clock (so `ctx.sleep(24.hours())` is instant) and
//! replays every workflow from its own history after it finishes, to catch non-determinism.
use crate::support::shop::*;
use crate::support::*;
use jackdaw::testing::TestEngine;

fn engine_for(shop: &Arc<Shop>) -> jackdaw::testing::TestEngineBuilder {
    TestEngine::builder().tasks(shop.clone())
}

// ---------------------------------------------------------------------------------------------------------------
// if / match are plain Rust
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Decision {
    AutoApproved,
    NeedsReview,
}

#[workflow(namespace = "rust-workflows", name = "triage", version = "1.0.0")]
pub struct Triage;

#[workflow_methods]
impl Triage {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Decision> {
        if input.total_cents > 100_000 {
            ctx.call(Shop::audit, &input).await?;
            Ok(Decision::NeedsReview)
        } else {
            Ok(Decision::AutoApproved)
        }
    }
}

#[tokio::test(start_paused = true)]
async fn branches_with_plain_rust_if() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Triage>().build()?;

    let small = engine
        .execute(Triage::run, Order::new("o-1", 500))
        .await?
        .await?;
    let large = engine
        .execute(Triage::run, Order::new("o-2", 500_000))
        .await?
        .await?;

    assert_eq!(small, Decision::AutoApproved);
    assert_eq!(large, Decision::NeedsReview);
    assert_eq!(shop.audits.get(), 1, "only the large order was audited");
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// for_each: the engine's `for` task, typed
// ---------------------------------------------------------------------------------------------------------------

#[workflow(namespace = "rust-workflows", name = "pricing", version = "1.0.0")]
pub struct Pricing;

#[workflow_methods]
impl Pricing {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Vec<Priced>> {
        Ok(ctx.for_each(&input.items, Shop::price).await?)
    }
}

#[workflow(
    namespace = "rust-workflows",
    name = "pricing-bounded",
    version = "1.0.0"
)]
pub struct PricingBounded;

#[workflow_methods]
impl PricingBounded {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Vec<Priced>> {
        Ok(ctx
            .for_each(&input.items, Shop::price)
            .concurrency(2)
            .await?)
    }
}

#[tokio::test(start_paused = true)]
async fn for_each_returns_one_typed_result_per_item_in_order() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Pricing>().build()?;

    let priced = engine
        .execute(Pricing::run, Order::new("o-1", 0))
        .await?
        .await?;

    let items: Vec<&str> = priced.iter().map(|p| p.item.as_str()).collect();
    assert_eq!(items, ["apple", "pear", "fig"]);
    assert_eq!(priced[0].cents, 500);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn for_each_concurrency_is_bounded() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<PricingBounded>().build()?;

    engine
        .execute(PricingBounded::run, Order::new("o-1", 0))
        .await?
        .await?;

    assert_eq!(shop.max_in_flight.load(Ordering::SeqCst), 2);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// join and select: steps in parallel
// ---------------------------------------------------------------------------------------------------------------

#[workflow(namespace = "rust-workflows", name = "both", version = "1.0.0")]
pub struct Both;

#[workflow_methods]
impl Both {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: Order,
    ) -> WorkflowResult<(Priced, Priced)> {
        // The standard futures combinators are fine: a step's identity is fixed when `ctx.call` is
        // invoked, not when it is polled, so replay does not depend on poll order.
        let (a, b) = futures::try_join!(
            ctx.call(Shop::price, input.items[0].clone()),
            ctx.call(Shop::price, input.items[1].clone()),
        )?;
        Ok((a, b))
    }
}

#[tokio::test(start_paused = true)]
async fn futures_join_runs_steps_in_parallel() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Both>().build()?;

    let (a, b) = engine
        .execute(Both::run, Order::new("o-1", 0))
        .await?
        .await?;

    assert_eq!((a.item.as_str(), b.item.as_str()), ("apple", "pear"));
    // Each price takes one virtual second. In parallel the workflow takes one second, not two.
    assert_eq!(engine.clock().elapsed(), 1.secs());
    Ok(())
}

#[workflow(namespace = "rust-workflows", name = "race", version = "1.0.0")]
pub struct Race;

#[workflow_methods]
impl Race {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<String> {
        // `select!` is the one combinator jackdaw provides: it journals which branch won, so replay agrees.
        Ok(jackdaw::select! {
            priced = ctx.call(Shop::price, input.items[0].clone()) => format!("priced {}", priced?.item),
            slept = ctx.sleep(500.millis()) => {
                slept?;
                "timed out".to_string()
            }
        })
    }
}

#[tokio::test(start_paused = true)]
async fn select_takes_the_first_branch_to_finish() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Race>().build()?;

    let winner = engine
        .execute(Race::run, Order::new("o-1", 0))
        .await?
        .await?;

    assert_eq!(
        winner, "timed out",
        "the sleep (500ms) beats the price call (1s)"
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Errors from steps are values you can match
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    Shipped(Shipment),
    Declined,
}

#[workflow(namespace = "rust-workflows", name = "checkout", version = "1.0.0")]
pub struct Checkout;

#[workflow_methods]
impl Checkout {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Outcome> {
        let receipt = match ctx.call(Shop::charge, &input).await {
            Ok(receipt) => receipt,
            // A declined card is business as usual: compensate and finish normally.
            Err(StepError::Fault(fault)) if fault.status == Some(402) => {
                ctx.call(Shop::refund, &input).await?;
                return Ok(Outcome::Declined);
            }
            // Anything else is not ours to handle: fault the workflow.
            Err(other) => return Err(other.into()),
        };
        Ok(Outcome::Shipped(ctx.call(Shop::ship, &receipt).await?))
    }
}

#[tokio::test(start_paused = true)]
async fn a_step_fault_can_be_matched_and_compensated() -> TestResult {
    let shop = Shop::failing_charges(402);
    let engine = engine_for(&shop).workflow::<Checkout>().build()?;

    let outcome = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert_eq!(outcome, Outcome::Declined);
    assert_eq!(shop.refunds.get(), 1);
    assert_eq!(shop.ships.get(), 0);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn an_unhandled_step_fault_faults_the_workflow() {
    let shop = Shop::failing_charges(500);
    let engine = engine_for(&shop).workflow::<Checkout>().build().unwrap();

    let err = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await
        .unwrap()
        .await
        .unwrap_err();

    let Error::Faulted(fault) = err else {
        panic!("expected a fault, got {err:?}");
    };
    assert_eq!(fault.status, Some(500));
    assert_eq!(shop.refunds.get(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_happy_path_ships() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Checkout>().build()?;

    let outcome = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert!(matches!(outcome, Outcome::Shipped(s) if s.order_id == "o-1"));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Step options: retry and timeout chain on any step
// ---------------------------------------------------------------------------------------------------------------

#[workflow(namespace = "rust-workflows", name = "patient", version = "1.0.0")]
pub struct Patient;

#[workflow_methods]
impl Patient {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Receipt> {
        Ok(ctx
            .call(Shop::charge, &input)
            .retry(Retry::exponential(5))
            .timeout(30.secs())
            .await?)
    }
}

#[tokio::test(start_paused = true)]
async fn step_options_apply_to_the_step() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Patient>().build()?;

    let receipt = engine
        .execute(Patient::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert_eq!(receipt.amount_cents, 500);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// State, signals, queries and waiting
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Waiting,
    Approved,
}

#[workflow(namespace = "rust-workflows", name = "approval", version = "1.0.0")]
pub struct Approval {
    approved: bool,
}

#[workflow_methods]
impl Approval {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { approved: false }
    }

    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Shipment> {
        let receipt = ctx.call(Shop::charge, &input).await?;
        ctx.wait_condition(|state| state.approved).await?;
        Ok(ctx.call(Shop::ship, &receipt).await?)
    }

    #[signal]
    fn approve(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.approved = true;
    }

    #[query]
    fn status(&self, _ctx: &WorkflowContextView) -> ApprovalStatus {
        if self.approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Waiting
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_signal_changes_state_and_unblocks_wait_condition() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Approval>().build()?;

    let run = engine
        .execute(Approval::run, Order::new("o-1", 500))
        .await?;
    assert_eq!(run.query(Approval::status).await?, ApprovalStatus::Waiting);
    assert_eq!(shop.ships.get(), 0, "nothing ships before approval");

    run.signal(Approval::approve, ()).await?;
    assert_eq!(run.query(Approval::status).await?, ApprovalStatus::Approved);

    let shipment = run.await?;
    assert_eq!(shipment.order_id, "o-1");
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaymentConfirmed {
    pub reference: String,
}

#[workflow(namespace = "rust-workflows", name = "awaits-event", version = "1.0.0")]
pub struct AwaitsEvent;

#[workflow_methods]
impl AwaitsEvent {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _input: Order) -> WorkflowResult<String> {
        let confirmed: PaymentConfirmed = ctx.wait_for_event("payment.confirmed").await?;
        Ok(confirmed.reference)
    }
}

#[tokio::test(start_paused = true)]
async fn wait_for_event_resumes_when_the_event_arrives() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<AwaitsEvent>().build()?;

    let run = engine
        .execute(AwaitsEvent::run, Order::new("o-1", 500))
        .await?;
    run.send_event(
        "payment.confirmed",
        PaymentConfirmed {
            reference: "pay-7".into(),
        },
    )
    .await?;

    assert_eq!(run.await?, "pay-7");
    Ok(())
}

#[workflow(namespace = "rust-workflows", name = "sleeps", version = "1.0.0")]
pub struct Sleeps;

#[workflow_methods]
impl Sleeps {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Receipt> {
        ctx.sleep(24.hours()).await?;
        Ok(ctx.call(Shop::charge, &input).await?)
    }
}

#[tokio::test(start_paused = true)]
async fn a_day_long_sleep_takes_no_real_time_under_test() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Sleeps>().build()?;

    engine
        .execute(Sleeps::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert!(engine.clock().elapsed() >= 24.hours());
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn cancelling_a_waiting_workflow_ends_it_as_cancelled() -> TestResult {
    let shop = Shop::new();
    let engine = engine_for(&shop).workflow::<Approval>().build()?;

    let run = engine
        .execute(Approval::run, Order::new("o-1", 500))
        .await?;
    let id = run.id().to_string();
    run.cancel().await?;

    assert!(matches!(run.await, Err(Error::Cancelled)));
    assert_eq!(engine.status(&id).await?, Status::Cancelled);
    assert_eq!(shop.ships.get(), 0);
    Ok(())
}
