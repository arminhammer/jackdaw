//! The shortest path from nothing to a result. If these read badly, the API is wrong.
use crate::support::*;

// A YAML workflow is run through the types you bind it to. Checked against the file's schemas when it has them,
// and otherwise when the values are read. (Code that cannot know the shape binds `Workflow<Value, Value>`.)
#[tokio::test]
async fn runs_a_yaml_workflow_bound_to_rust_types() -> TestResult {
    let engine = Engine::builder().build()?;
    let workflow = Workflow::<Name, Greeting>::from_yaml(HELLO_YAML)?;

    let greeting: Greeting = engine.execute(&workflow, Name::new("Ada")).await?.await?;

    assert_eq!(greeting, Greeting::hello("Ada"));
    Ok(())
}

// A workflow written entirely in Rust. `#[run]` is ordinary async Rust; `ctx.call` runs a typed task.
#[workflow(namespace = "examples", name = "hello-rust", version = "1.0.0")]
pub struct HelloWorkflow;

#[workflow_methods]
impl HelloWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Name) -> WorkflowResult<Greeting> {
        Ok(ctx.call(Greet, &input).await?)
    }
}

#[tokio::test]
async fn runs_a_workflow_written_in_rust() -> TestResult {
    let engine = Engine::builder()
        .task(Greet)
        .workflow::<HelloWorkflow>()
        .build()?;

    let greeting: Greeting = engine
        .execute(HelloWorkflow::run, Name::new("Ada"))
        .await?
        .await?;

    assert_eq!(greeting, Greeting::hello("Ada"));
    Ok(())
}

#[tokio::test]
async fn the_handle_exposes_the_instance_id() -> TestResult {
    let engine = Engine::builder()
        .task(Greet)
        .workflow::<HelloWorkflow>()
        .build()?;

    let run = engine.execute(HelloWorkflow::run, Name::new("Ada")).await?;
    let id = run.id().to_string();
    run.await?;

    assert!(!id.is_empty());
    assert_eq!(engine.status(&id).await?, Status::Completed);
    Ok(())
}
