//! A workflow written in Rust that calls two tasks, on an in-memory engine.
//!
//! ```text
//! cargo run -p jackdaw-wf --example quickstart
//! ```
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Order {
    pub id: String,
    pub cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Receipt {
    pub order: String,
    pub charge: String,
    pub parcel: String,
}

/// A task: ordinary async Rust. Its result is journaled, so it does not run again on replay.
#[task]
async fn charge(order: Order) -> Result<String, TaskError> {
    Ok(format!("ch_{}_{}", order.id, order.cents))
}

#[task]
async fn ship(order: Order) -> Result<String, TaskError> {
    Ok(format!("parcel-for-{}", order.id))
}

#[workflow(namespace = "examples", name = "checkout", version = "1.0.0")]
pub struct Checkout;

#[workflow_methods]
impl Checkout {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Receipt> {
        let charge = ctx
            .call(Charge, &order)
            .retry(Retry::exponential(3))
            .await?;
        let parcel = ctx.call(Ship, &order).await?;
        Ok(Receipt {
            order: order.id,
            charge,
            parcel,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder()
        .task(Charge)
        .task(Ship)
        .workflow::<Checkout>()
        .build()?;

    let receipt = engine
        .execute(
            Checkout::run,
            Order {
                id: "o-1".into(),
                cents: 4200,
            },
        )
        .await?
        .await?;

    println!("{receipt:?}");
    assert_eq!(receipt.parcel, "parcel-for-o-1");
    Ok(())
}
