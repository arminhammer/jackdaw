//! durability/redb_upgrade.feature: Redb files written by an earlier release.
use super::world::LibWorld;
use cucumber::{given, then, when};
use jackdaw::api::Store;
use jackdaw::prelude::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The instance the fixture left waiting for its signal, and the one it left a lease and a message for.
const WAITING: &str = "upgrade-1";
const LEASED: &str = "upgrade-leased";

// The workflow of `tests/fixtures/redb4/generator`, which wrote the files: its names and types must stay the same. The
// task counts its runs, to show what the stored journal and cache save from running again.

pub struct Upgrade(Arc<AtomicUsize>);

#[tasks]
impl Upgrade {
    #[task(name = "upgrade.prepare")]
    pub async fn prepare(
        self: Arc<Self>,
        _ctx: TaskContext,
        order: u32,
    ) -> Result<String, TaskError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(format!("order {order}"))
    }
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
        let prepared = ctx.call(Upgrade::prepare, &order).cached().await?;
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

/// The scenario's copy of a fixture, and what was opened from it.
#[derive(Default)]
pub struct State {
    path: PathBuf,
    store: Option<Redb>,
    engine: Option<Engine>,
    runs: Arc<AtomicUsize>,
}

impl State {
    fn engine(&mut self) -> Engine {
        let store = self.store.clone().expect("the database is open");
        let runs = Arc::clone(&self.runs);
        self.engine
            .get_or_insert_with(|| {
                Engine::builder()
                    .store(store)
                    .tasks(Arc::new(Upgrade(runs)))
                    .workflow::<Approval>()
                    .build()
                    .expect("the engine builds")
            })
            .clone()
    }
}

#[given(
    regex = r"^the Redb database written by jackdaw 0\.3\.0 on redb 4 and (closed cleanly|left open by a killed process)$"
)]
fn written_by_redb_4(world: &mut LibWorld, state: String) {
    let name = if state == "closed cleanly" {
        "closed.redb"
    } else {
        "killed.redb"
    };
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/redb4")
        .join(name);
    world.upgrade.path = world.dir.path().join(name);
    // A copy: opening a database writes to it.
    std::fs::copy(&fixture, &world.upgrade.path).expect("copy the fixture");
}

#[when("the current release opens it")]
fn open_it(world: &mut LibWorld) {
    world.upgrade.store =
        Some(Redb::open(&world.upgrade.path).expect("a database written on redb 4 opens"));
}

#[then("the history, the lease and the message it holds are all there")]
async fn everything_is_there(world: &mut LibWorld) {
    let persistence = world
        .upgrade
        .store
        .as_ref()
        .expect("the database is open")
        .persistence();

    let events = persistence.get_events(WAITING).await.expect("the events");
    assert_eq!(events.len(), 3, "{events:?}");
    let journal = persistence
        .load_journal(WAITING)
        .await
        .expect("the journal");
    assert!(!journal.is_empty(), "the journal is empty");

    // Asking for a lease someone else holds returns theirs.
    let now = chrono::Utc::now();
    let lease = persistence
        .acquire_lease(
            LEASED,
            "this-engine",
            now,
            now + chrono::Duration::seconds(30),
        )
        .await
        .expect("read the lease");
    assert_eq!(lease.owner, "an-engine-on-redb-4");

    // Messages go to the lease's owner.
    let messages = persistence
        .take_messages("an-engine-on-redb-4")
        .await
        .expect("the messages");
    assert_eq!(
        messages,
        vec![(LEASED.to_string(), json!({"note": "left by redb 4"}))]
    );
}

#[then(
    "its waiting instance resumes, takes its signal and completes without running its first step again"
)]
async fn resumes(world: &mut LibWorld) {
    let engine = world.upgrade.engine();
    engine.recover().await.expect("recover");
    let handle = engine
        .handle::<Approval>(WAITING)
        .await
        .expect("a handle on the instance");
    handle
        .signal(Approval::approve, "sam".to_string())
        .await
        .expect("signal");
    let result = tokio::time::timeout(Duration::from_secs(30), handle)
        .await
        .expect("the instance completes in time")
        .expect("the instance completes");
    assert_eq!(result, "order 7 approved by sam");
    assert_eq!(world.upgrade.runs.load(Ordering::SeqCst), 0);
}

#[then("a new instance with the same input gets the first step's result from the cache")]
async fn cached(world: &mut LibWorld) {
    let engine = world.upgrade.engine();
    let handle = engine
        .execute(Approval::run, 7)
        .await
        .expect("a new instance");
    handle
        .signal(Approval::approve, "kim".to_string())
        .await
        .expect("signal");
    let result = tokio::time::timeout(Duration::from_secs(30), handle)
        .await
        .expect("the instance completes in time")
        .expect("the instance completes");
    assert_eq!(result, "order 7 approved by kim");
    assert_eq!(world.upgrade.runs.load(Ordering::SeqCst), 0);
}
