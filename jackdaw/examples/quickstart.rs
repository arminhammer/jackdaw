//! A workflow written in Rust that calls two tasks, run as a program. Its input, `Order`, is its configuration: it
//! comes from flags, the `CHECKOUT_` environment variables, or `--config` files.
//!
//! ```text
//! cargo run -p jackdaw-wf --example quickstart -- --id o-1 --cents 4200
//! cargo run -p jackdaw-wf --example quickstart -- --help
//! ```
use jackdaw::prelude::*;
use jackdaw::runner::Program;
use serde::{Deserialize, Serialize};

/// The workflow's input and, with `#[derive(Config)]`, its command line.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Config)]
pub struct Order {
    /// The order to ship
    pub id: String,
    /// What to charge, in cents
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
async fn main() -> Result<std::process::ExitCode, Error> {
    let engine = Engine::builder()
        .task(Charge)
        .task(Ship)
        .workflow::<Checkout>()
        .build()?;
    // Parses the flags, reads the input, runs the workflow and prints its output.
    Ok(Program::<Checkout>::new(engine).main().await)
}
