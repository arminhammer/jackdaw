//! Workflows nest: a workflow runs another workflow, typed, whether the child is Rust or YAML, and Rust and YAML
//! can call each other in both directions. (A YAML workflow can call Rust *tasks* by name, see `tasks.rs`; it cannot
//! run a Rust *workflow* as a child.)
use crate::support::shop::*;
use crate::support::*;
use jackdaw::testing::TestEngine;

// A child workflow: charge and ship an order.
#[workflow(namespace = "nesting", name = "fulfil", version = "1.0.0")]
pub struct Fulfil;

#[workflow_methods]
impl Fulfil {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Shipment> {
        let receipt = ctx.call(Shop::charge, &input).await?;
        Ok(ctx.call(Shop::ship, &receipt).await?)
    }
}

// A parent that runs the child like any other step; `ctx.run` is typed by the child's input and output.
#[workflow(namespace = "nesting", name = "checkout", version = "1.0.0")]
pub struct Checkout;

#[workflow_methods]
impl Checkout {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Shipment> {
        ctx.call(Shop::audit, &input).await?;
        Ok(ctx.run(Fulfil::run, &input).await?)
    }
}

#[tokio::test(start_paused = true)]
async fn a_workflow_runs_a_child_workflow_with_typed_input_and_output() -> TestResult {
    let shop = Shop::new();
    let engine = TestEngine::builder()
        .tasks(shop.clone())
        .workflow::<Checkout>()
        .build()?;

    let shipment: Shipment = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert_eq!(shipment.order_id, "o-1");
    assert_eq!(
        (shop.audits.get(), shop.charges.get(), shop.ships.get()),
        (1, 1, 1)
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn registering_a_parent_registers_the_children_it_runs() -> TestResult {
    // `Fulfil` is never registered by name: `Checkout` runs it, so registering `Checkout` is enough.
    let engine = TestEngine::builder()
        .tasks(Shop::new())
        .workflow::<Checkout>()
        .build()?;

    engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await?
        .await?;
    Ok(())
}

// A child can also be run on its own.
#[tokio::test(start_paused = true)]
async fn a_child_workflow_can_run_on_its_own() -> TestResult {
    let engine = TestEngine::builder()
        .tasks(Shop::new())
        .workflow::<Fulfil>()
        .build()?;

    let shipment = engine
        .execute(Fulfil::run, Order::new("o-2", 500))
        .await?
        .await?;

    assert_eq!(shipment.order_id, "o-2");
    Ok(())
}

// A child's failure reaches the parent as a step error the parent can handle.
#[workflow(namespace = "nesting", name = "tolerant", version = "1.0.0")]
pub struct Tolerant;

#[workflow_methods]
impl Tolerant {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: Order,
    ) -> WorkflowResult<Option<Shipment>> {
        match ctx.run(Fulfil::run, &input).await {
            Ok(shipment) => Ok(Some(shipment)),
            Err(StepError::Fault(fault)) if fault.status == Some(402) => Ok(None),
            Err(other) => Err(other.into()),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_child_fault_can_be_handled_by_the_parent() -> TestResult {
    let engine = TestEngine::builder()
        .tasks(Shop::failing_charges(402))
        .workflow::<Tolerant>()
        .build()?;

    let result = engine
        .execute(Tolerant::run, Order::new("o-1", 500))
        .await?
        .await?;

    assert_eq!(result, None);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn an_unhandled_child_fault_faults_the_parent() {
    let engine = TestEngine::builder()
        .tasks(Shop::failing_charges(500))
        .workflow::<Checkout>()
        .build()
        .unwrap();

    let err = engine
        .execute(Checkout::run, Order::new("o-1", 500))
        .await
        .unwrap()
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Faulted(f) if f.status == Some(500)));
}

// ---------------------------------------------------------------------------------------------------------------
// Rust runs YAML
// ---------------------------------------------------------------------------------------------------------------

const SHIPPING_LABEL_YAML: &str = r#"
document: { dsl: '1.0.3', namespace: nesting, name: label, version: '1.0.0' }
do:
  - label:
      set:
        text: '${ "SHIP TO: " + .order_id }'
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Label {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LabelRequest {
    pub order_id: String,
}

#[workflow(namespace = "nesting", name = "labelled", version = "1.0.0")]
pub struct Labelled;

#[workflow_methods]
impl Labelled {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Order) -> WorkflowResult<Label> {
        // A YAML workflow bound to Rust types, run as a child step.
        let label = Workflow::<LabelRequest, Label>::from_yaml(SHIPPING_LABEL_YAML)?;
        let request = LabelRequest { order_id: input.id };
        Ok(ctx.run_definition(&label, &request).await?)
    }
}

#[tokio::test(start_paused = true)]
async fn rust_can_run_a_yaml_workflow_as_a_typed_child() -> TestResult {
    let engine = TestEngine::builder().workflow::<Labelled>().build()?;

    let label = engine
        .execute(Labelled::run, Order::new("o-9", 1))
        .await?
        .await?;

    assert_eq!(label.text, "SHIP TO: o-9");
    Ok(())
}
