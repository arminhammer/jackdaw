//! A durable sleep: the timer fires on real time, whatever the host's clock does.
use jackdaw::prelude::*;
use std::time::{Duration, Instant};

#[workflow(namespace = "examples", name = "sleeper", version = "1.0.0")]
struct Sleeper;

#[workflow_methods]
impl Sleeper {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        ctx.sleep(1.secs()).await?;
        Ok("awake".to_string())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder().workflow::<Sleeper>().build()?;
    let started = Instant::now();
    let result = engine.execute(Sleeper::run, ()).await?.await?;
    println!(
        "{result}, after at least 1s: {}",
        started.elapsed() >= Duration::from_secs(1)
    );
    Ok(())
}
