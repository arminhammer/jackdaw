//! A workflow that survives the program stopping: the first run stops while a timer is pending, the second recovers
//! from the Redb file and finishes without charging again.
//!
//! Arguments: a directory the host gave this program access to, and `start` or `finish`.
use jackdaw::prelude::*;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

static DIR: OnceLock<PathBuf> = OnceLock::new();

fn effects() -> PathBuf {
    DIR.get().expect("the directory is set").join("effects.log")
}

#[task]
async fn charge(order: String) -> Result<String, TaskError> {
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(effects())
        .map_err(TaskError::non_retryable)?;
    writeln!(log, "charged {order}").map_err(TaskError::non_retryable)?;
    Ok(order)
}

#[task]
async fn ship(order: String) -> Result<String, TaskError> {
    let charges = std::fs::read_to_string(effects())
        .map(|log| log.lines().count())
        .map_err(TaskError::non_retryable)?;
    Ok(format!("shipped {order} after charging {charges} time(s)"))
}

#[workflow(namespace = "examples", name = "durable", version = "1.0.0")]
struct Durable;

#[workflow_methods]
impl Durable {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: String) -> WorkflowResult<String> {
        let order = ctx.call(Charge, &order).await?;
        ctx.sleep(2.secs()).await?;
        Ok(ctx.call(Ship, &order).await?)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let mut arguments = std::env::args().skip(1);
    let dir = PathBuf::from(arguments.next().expect("a directory"));
    let mode = arguments.next().expect("start or finish");
    let engine = Engine::builder()
        .store(Redb::open(dir.join("jackdaw.redb"))?)
        .task(Charge)
        .task(Ship)
        .workflow::<Durable>()
        .build()?;
    DIR.set(dir).expect("set once");

    match mode.as_str() {
        "start" => {
            let _run = engine
                .execute_with_id("order-1", Durable::run, "o-1".to_string())
                .await?;
            // The charge is journaled once its completion is in the history; the timer is pending after that.
            while !engine
                .history("order-1")
                .await?
                .iter()
                .any(|event| event.name() == "TaskCompleted")
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            println!("stopped during the timer");
            std::process::exit(0);
        }
        _ => {
            engine.recover().await?;
            let shipped = engine.handle::<Durable>("order-1").await?.await?;
            println!("{shipped}");
        }
    }
    Ok(())
}
