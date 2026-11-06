//! A blocking task, and a limit on async calls that wait: both on the single thread wasm has.
use jackdaw::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static RUNNING: AtomicUsize = AtomicUsize::new(0);
static MOST: AtomicUsize = AtomicUsize::new(0);

#[task(blocking, limit = "cpu")]
fn checksum(data: Vec<u8>) -> Result<u32, TaskError> {
    Ok(data.iter().map(|byte| u32::from(*byte)).sum())
}

// Waits inside the call, so without the limit the calls would overlap.
#[task(limit = "render")]
async fn render(n: u32) -> Result<u32, TaskError> {
    let running = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
    MOST.fetch_max(running, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(10)).await;
    RUNNING.fetch_sub(1, Ordering::SeqCst);
    Ok(n)
}

#[workflow(namespace = "examples", name = "blocking", version = "1.0.0")]
struct Blocking;

#[workflow_methods]
impl Blocking {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<(u32, usize)> {
        let sum = ctx.call(Checksum, &vec![1, 2, 3, 4]).await?;
        let rendered = ctx
            .for_each(&[1u32, 2, 3, 4], Render)
            .concurrency(4)
            .await?;
        Ok((sum, rendered.len()))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder()
        .task(Checksum)
        .task(Render)
        .limit("cpu", 2)
        .limit("render", 1)
        .workflow::<Blocking>()
        .build()?;
    let (sum, rendered) = engine.execute(Blocking::run, ()).await?.await?;
    println!(
        "checksum={sum} rendered={rendered} most_at_once={}",
        MOST.load(Ordering::SeqCst)
    );
    Ok(())
}
