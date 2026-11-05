//! The README's example on the native API, with the default in-memory store.
use jackdaw::prelude::*;

#[task]
async fn greet(name: String) -> Result<String, TaskError> {
    Ok(format!("Hello, {name}!"))
}

#[workflow(namespace = "examples", name = "hello", version = "1.0.0")]
struct Hello;

#[workflow_methods]
impl Hello {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, name: String) -> WorkflowResult<String> {
        Ok(ctx.call(Greet, &name).retry(Retry::exponential(3)).await?)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder().task(Greet).workflow::<Hello>().build()?;
    let greeting = engine
        .execute(Hello::run, "World".to_string())
        .await?
        .await?;
    println!("{greeting}");
    Ok(())
}
