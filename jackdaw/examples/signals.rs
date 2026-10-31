//! Signals change a running workflow's state, queries read it, and `wait_condition` waits for it.
//!
//! ```text
//! cargo run -p jackdaw-wf --example signals
//! ```
use jackdaw::prelude::*;

#[workflow(namespace = "examples", name = "cart", version = "1.0.0")]
pub struct Cart {
    items: Vec<String>,
    checked_out: bool,
}

#[workflow_methods]
impl Cart {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self {
            items: Vec::new(),
            checked_out: false,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<String>> {
        // Durable: the wait survives a restart, and a week passes on the engine's clock, not in memory.
        let checked_out = jackdaw::select! {
            _ = ctx.wait_condition(|cart| cart.checked_out) => true,
            _ = ctx.sleep(7.days()) => false,
        };
        Ok(if checked_out {
            ctx.state(|cart| cart.items.clone())
        } else {
            Vec::new()
        })
    }

    #[signal]
    fn add(&mut self, _ctx: &mut SyncWorkflowContext<Self>, item: String) {
        self.items.push(item);
    }

    #[signal]
    fn checkout(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.checked_out = true;
    }

    #[query]
    fn items(&self, _ctx: &WorkflowContextView) -> Vec<String> {
        self.items.clone()
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder().workflow::<Cart>().build()?;
    let cart = engine.execute(Cart::run, ()).await?;

    cart.signal(Cart::add, "book".to_string()).await?;
    cart.signal(Cart::add, "lamp".to_string()).await?;
    println!("in the cart: {:?}", cart.query(Cart::items).await?);

    cart.signal(Cart::checkout, ()).await?;
    let bought = cart.await?;
    println!("bought: {bought:?}");
    assert_eq!(bought, ["book", "lamp"]);
    Ok(())
}
