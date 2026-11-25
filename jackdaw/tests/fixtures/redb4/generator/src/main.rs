//! Writes the redb 4 store fixtures of `tests/lib/durability/redb_upgrade.feature` with the released jackdaw 0.3.0.
//! The workflow must stay the same as the one in `tests/lib_steps/redb_upgrade.rs`.
//!
//! `cargo run -- <output directory>` writes `closed.redb` (closed cleanly) and `killed.redb` (copied while the store
//! was still open, which is what a killed process leaves behind).
use jackdaw::api::Store as _;
use jackdaw::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[task(name = "upgrade.prepare")]
async fn prepare(order: u32) -> Result<String, TaskError> {
    Ok(format!("order {order}"))
}

#[workflow(namespace = "upgrade", name = "approval", version = "1")]
pub struct Approval {
    approver: Option<String>,
}

#[workflow_methods]
impl Approval {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { approver: None }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: u32) -> WorkflowResult<String> {
        let prepared = ctx.call(Prepare, order).cached().await?;
        ctx.wait_condition(|approval| approval.approver.is_some())
            .await?;
        let approver = ctx
            .state(|approval| approval.approver.clone())
            .unwrap_or_default();
        Ok(format!("{prepared} approved by {approver}"))
    }

    #[signal]
    fn approve(&mut self, _ctx: &mut SyncWorkflowContext<Self>, approver: String) {
        self.approver = Some(approver);
    }
}

async fn write(path: &Path, killed_copy: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let store = Redb::open(path)?;
    let engine = Engine::builder()
        .store(store.clone())
        .task(Prepare)
        .workflow::<Approval>()
        .build()?;
    let _run = engine
        .execute_with_id("upgrade-1", Approval::run, 7)
        .await?;
    // The first step is done and the instance is waiting for its signal.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let persistence = store.persistence();
    let now = chrono::Utc::now();
    persistence
        .acquire_lease(
            "upgrade-leased",
            "an-engine-on-redb-4",
            now,
            "2100-01-01T00:00:00Z".parse()?,
        )
        .await?;
    persistence
        .post_message(
            "upgrade-leased",
            serde_json::json!({"note": "left by redb 4"}),
        )
        .await?;
    println!("history: {:#?}", engine.history("upgrade-1").await?);
    println!("status: {:?}", engine.status("upgrade-1").await?);
    if let Some(copy) = killed_copy {
        std::fs::copy(path, copy)?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("give the output directory")?);
    std::fs::create_dir_all(&out)?;
    let scratch = out.join("scratch.redb");
    write(&out.join("closed.redb"), None).await?;
    write(&scratch, Some(&out.join("killed.redb"))).await?;
    std::fs::remove_file(scratch)?;
    Ok(())
}
