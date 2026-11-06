//! Calls that run at once, a bounded `for_each` and a retry, on the single thread wasm has.
use jackdaw::prelude::*;

#[task]
async fn price(sku: String) -> Result<u64, TaskError> {
    Ok(sku.len() as u64 * 10)
}

#[task]
async fn stock(sku: String) -> Result<u32, TaskError> {
    Ok(sku.len() as u32)
}

#[task]
async fn double(n: u32) -> Result<u32, TaskError> {
    Ok(n * 2)
}

// Fails twice, then works: the third attempt is the first to succeed.
#[task]
async fn flaky(ctx: TaskContext, n: u32) -> Result<u32, TaskError> {
    if ctx.attempt() < 3 {
        Err(TaskError::retryable("not yet"))
    } else {
        Ok(n * 2)
    }
}

#[workflow(namespace = "examples", name = "fan-out", version = "1.0.0")]
struct FanOut;

#[workflow_methods]
impl FanOut {
    #[run]
    async fn run(
        ctx: &mut WorkflowContext<Self>,
        sku: String,
    ) -> WorkflowResult<(u64, u32, Vec<u32>, u32)> {
        let (price, stock) = futures::try_join!(ctx.call(Price, &sku), ctx.call(Stock, &sku))?;
        let doubled = ctx.for_each(&[1u32, 2, 3], Double).concurrency(2).await?;
        let flaky = ctx.call(Flaky, &21).retry(Retry::exponential(3)).await?;
        Ok((price, stock, doubled, flaky))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder()
        .task(Price)
        .task(Stock)
        .task(Double)
        .task(Flaky)
        .workflow::<FanOut>()
        .build()?;
    let (price, stock, doubled, flaky) = engine
        .execute(FanOut::run, "abc".to_string())
        .await?
        .await?;
    println!("price={price} stock={stock} doubled={doubled:?} flaky={flaky}");
    Ok(())
}
