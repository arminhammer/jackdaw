//! A workflow survives its engine: the first engine stops mid-run, a second one on the same store finishes it, and
//! the task that already ran is not run again.
//!
//! ```text
//! cargo run -p jackdaw --example durable_restart
//! ```
use jackdaw::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static REPORTS: AtomicUsize = AtomicUsize::new(0);

#[task]
async fn report(day: u32) -> Result<String, TaskError> {
    REPORTS.fetch_add(1, Ordering::SeqCst);
    Ok(format!("report for day {day}"))
}

#[workflow(namespace = "examples", name = "two-reports", version = "1.0.0")]
pub struct TwoReports;

#[workflow_methods]
impl TwoReports {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<Vec<String>> {
        let first = ctx.call(Report, 1).await?;
        // A durable timer: its deadline is in the store, so a restart does not start it over.
        ctx.sleep(Duration::from_millis(300)).await?;
        let second = ctx.call(Report, 2).await?;
        Ok(vec![first, second])
    }
}

fn engine(path: &std::path::Path) -> Result<Engine, Error> {
    Engine::builder()
        .store(Redb::open(path)?)
        .task(Report)
        .workflow::<TwoReports>()
        .build()
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let dir = tempfile::tempdir().map_err(|e| Error::Engine(e.to_string()))?;
    let path = dir.path().join("jackdaw.redb");

    let first = engine(&path)?;
    let id = first.execute(TwoReports::run, ()).await?.id().to_string();
    // Let the first report run, then "crash": the engine and its instances are dropped mid-sleep.
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(first);
    // The store's file is released once the dropped engine's tasks have stopped.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let second = engine(&path)?;
    let recovered = second.recover().await?;
    println!("resumed: {:?}", recovered.resumed);
    let reports = second.handle::<TwoReports>(&id).await?.await?;

    println!("{reports:?}");
    assert_eq!(REPORTS.load(Ordering::SeqCst), 2, "each report ran once");
    Ok(())
}
