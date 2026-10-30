//! workflows/batch_fan_out.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Job {
    pub chunks: u32,
    pub parallelism: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    pub chunks: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    pub imported: usize,
}

pub struct Batch(pub Arc<Probe>);

#[tasks]
impl Batch {
    #[task(name = "batch.split")]
    pub async fn split(self: Arc<Self>, ctx: TaskContext, job: Job) -> Result<Manifest, TaskError> {
        self.0.typed("split", &ctx, &job).await
    }
    #[task(name = "batch.import_chunk")]
    pub async fn import_chunk(
        self: Arc<Self>,
        ctx: TaskContext,
        chunk: u32,
    ) -> Result<u32, TaskError> {
        self.0.typed("import_chunk", &ctx, &chunk).await
    }
    #[task(name = "batch.rollback")]
    pub async fn rollback(self: Arc<Self>, ctx: TaskContext, job: Job) -> Result<(), TaskError> {
        self.0.typed("rollback", &ctx, &job).await
    }
    #[task(name = "batch.aggregate")]
    pub async fn aggregate(
        self: Arc<Self>,
        ctx: TaskContext,
        ok: Vec<u32>,
    ) -> Result<Report, TaskError> {
        self.0.typed("aggregate", &ctx, &ok).await
    }
    #[task(name = "batch.notify_report")]
    pub async fn notify_report(
        self: Arc<Self>,
        ctx: TaskContext,
        report: Report,
    ) -> Result<(), TaskError> {
        self.0.typed("notify_report", &ctx, &report).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("split", |job| {
        let n = job["chunks"].as_u64().unwrap_or(0);
        json!({ "chunks": (0..n).collect::<Vec<_>>() })
    });
    probe.default_output("import_chunk", |chunk| chunk.clone());
    probe.default_output("rollback", |_| json!(null));
    probe.default_output(
        "aggregate",
        |ok| json!({ "imported": ok.as_array().map_or(0, Vec::len) }),
    );
    probe.default_output("notify_report", |_| json!(null));
}

#[workflow(namespace = "data", name = "notify", version = "1.0.0")]
pub struct Notify;

#[workflow_methods]
impl Notify {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, report: Report) -> WorkflowResult<()> {
        Ok(ctx.call(Batch::notify_report, &report).await?)
    }
}

#[workflow(namespace = "data", name = "import", version = "1.0.0")]
pub struct Import;

#[workflow_methods]
impl Import {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, job: Job) -> WorkflowResult<Report> {
        let manifest = ctx.call(Batch::split, &job).await?;
        let (mut ok, mut failed) = (Vec::new(), Vec::new());
        for wave in manifest.chunks.chunks(job.parallelism) {
            let results =
                futures::future::join_all(wave.iter().map(|c| ctx.call(Batch::import_chunk, c)))
                    .await;
            for (chunk, result) in wave.iter().zip(results) {
                match result {
                    Ok(r) => ok.push(r),
                    Err(_) => failed.push(*chunk),
                }
            }
        }
        if failed.len() * 10 > manifest.chunks.len() {
            ctx.call(Batch::rollback, &job).await?;
            return Err(TaskError::fault("validation")
                .title("too many failed chunks")
                .into());
        }
        for chunk in &failed {
            ok.push(
                ctx.call(Batch::import_chunk, chunk)
                    .retry(Retry::exponential(3))
                    .await?,
            );
        }
        let report = ctx.call(Batch::aggregate, &ok).await?;
        ctx.run(Notify::run, &report).await?;
        Ok(report)
    }
}

#[workflow(namespace = "data", name = "import-strict", version = "1.0.0")]
pub struct ImportStrict;

#[workflow_methods]
impl ImportStrict {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, job: Job) -> WorkflowResult<Report> {
        let manifest = ctx.call(Batch::split, &job).await?;
        let ok = ctx
            .for_each(&manifest.chunks, Batch::import_chunk)
            .concurrency(8)
            .await?;
        Ok(ctx.call(Batch::aggregate, &ok).await?)
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Notify>("batch.notify"),
        Fixture::of::<Import>("batch.import").input(json!({"chunks": 40, "parallelism": 8})),
        Fixture::of::<ImportStrict>("batch.import-strict")
            .input(json!({"chunks": 40, "parallelism": 8})),
    ]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![("struct Import;", "batch.import")]
}

#[when(regex = r#"^I execute "Import"(?: with (\d+) chunks)?(?: and parallelism (\d+))?$"#)]
async fn execute_import(world: &mut LibWorld, chunks: String, parallelism: String) {
    let chunks: u32 = chunks.parse().unwrap_or(40);
    let parallelism: usize = parallelism.parse().unwrap_or(8);
    world.probe.script("import_chunk", |s| {
        s.delay = s.delay.or(Some(Duration::from_millis(100)))
    });
    world
        .start_fixture(
            "it",
            super::fixture("batch.import"),
            json!({"chunks": chunks, "parallelism": parallelism}),
        )
        .await;
}

#[then(regex = r"^the workflow completes with (?:a report of )?(\d+) imported chunks$")]
async fn imported(world: &mut LibWorld, n: usize) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!({ "imported": n }));
}

#[then(regex = r#"^at most (\d+) "ImportChunk" calls ran at once$"#)]
async fn at_most_chunks(world: &mut LibWorld, n: usize) {
    world.settle().await;
    let peak = world.probe.max_in_flight("import_chunk");
    assert!(peak <= n && peak > 1, "{peak} at once");
}

#[then(regex = r#"^the "Notify" child workflow ran once$"#)]
async fn notify_once(world: &mut LibWorld) {
    world.settle().await;
    assert_eq!(world.probe.count("notify_report"), 1);
}

#[given(regex = r"^chunks (\d+) and (\d+) fail on their first attempt only$")]
fn chunks_fail_once(world: &mut LibWorld, a: u32, b: u32) {
    world.probe.script("import_chunk", |s| {
        for chunk in [a, b] {
            s.fail_on
                .push((json!(chunk), FaultSpec::retryable("flaky chunk"), Some(1)));
        }
    });
}

#[given(regex = r"^(\d+) of (\d+) chunks fail every time$")]
fn chunks_fail_always(world: &mut LibWorld, n: u32, _of: u32) {
    world.probe.script("import_chunk", |s| {
        for chunk in 0..n {
            s.fail_on.push((
                json!(chunk * 5),
                FaultSpec::non_retryable("bad chunk"),
                None,
            ));
        }
    });
}

#[then(regex = r#"^the workflow fails with "([^"]+)"$"#)]
async fn fails_with_title(world: &mut LibWorld, title: String) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.title.as_deref(), Some(title.as_str()));
}

#[given(regex = r"^the engine is dropped after (\d+) of (\d+) chunks finished$")]
async fn dropped_after_chunks(world: &mut LibWorld, n: usize, total: u32) {
    world
        .probe
        .script("import_chunk", |s| s.block_on = Some(json!(26)));
    world
        .start_fixture(
            "it",
            super::fixture("batch.import"),
            json!({"chunks": total, "parallelism": 8}),
        )
        .await;
    let probe = Arc::clone(&world.probe);
    world
        .until("enough chunks finished", || {
            probe.completed("import_chunk") >= n
        })
        .await;
    world.crash().await;
    world.probe.release_all();
}

#[then("each chunk was imported exactly once in total")]
async fn each_chunk_once(world: &mut LibWorld) {
    let last = world.last();
    world.outcome(last).await.value();
    for chunk in 0..40u32 {
        let done = world
            .probe
            .calls_of("import_chunk")
            .iter()
            .filter(|c| c.input == json!(chunk) && !c.cancelled)
            .count();
        assert_eq!(done, 1, "chunk {chunk}");
    }
}

#[given(
    regex = r#"^a variant of "Import" that does "ctx\.for_each\(.+\)\.concurrency\(8\)\.await\?"$"#
)]
fn strict_variant(world: &mut LibWorld) {
    world.set_fixture(super::fixture("batch.import-strict"));
}

#[given(regex = r"^chunk (\d+) fails non-retryably$")]
fn chunk_fails(world: &mut LibWorld, chunk: u32) {
    world.probe.script("import_chunk", |s| {
        s.fail_on.push((
            json!(chunk),
            FaultSpec::non_retryable(&format!("chunk {chunk} is broken")),
            None,
        ));
    });
}

#[when(regex = r"^I execute it with (\d+) chunks$")]
async fn execute_with_chunks(world: &mut LibWorld, chunks: u32) {
    world
        .start("it", json!({"chunks": chunks, "parallelism": 8}))
        .await;
}

#[then(regex = r"^the workflow fails with chunk (\d+)'s fault$")]
async fn fails_with_chunk(world: &mut LibWorld, chunk: u32) {
    let last = world.last();
    let fault = world.outcome(last).await.fault().clone();
    assert_eq!(fault.detail, Some(format!("chunk {chunk} is broken")));
}

#[when(regex = r#"^I execute "Import" with (\d+) chunks and parallelism (\d+) on a Redb store$"#)]
async fn execute_large(world: &mut LibWorld, chunks: u32, parallelism: usize) {
    // The scenario measures the embedded store, whatever store the rest of the suite runs on.
    assert!(
        world.engine.is_none(),
        "the engine is built after the store is chosen"
    );
    world.store = Some("redb".into());
    world.durable = true;
    let started = Instant::now();
    world
        .start_fixture(
            "it",
            super::fixture("batch.import"),
            json!({"chunks": chunks, "parallelism": parallelism}),
        )
        .await;
    let last = world.last();
    world
        .outcome_within(last, Duration::from_secs(120))
        .await
        .value();
    world
        .notes
        .insert("took".into(), json!(started.elapsed().as_secs_f64()));
    world.notes.insert("chunks".into(), json!(chunks));
}

#[then(regex = r"^it completes in under (\d+) seconds with no-op chunk tasks$")]
fn completes_under(world: &mut LibWorld, seconds: f64) {
    let took = world.notes["took"].as_f64().expect("timed");
    assert!(took < seconds, "took {took}s");
}

#[then("the journal size grows linearly with the number of chunks")]
fn journal_linear(world: &mut LibWorld) {
    let chunks = world.notes["chunks"].as_u64().expect("chunks");
    let size = std::fs::metadata(world.dir.path().join("jackdaw.redb"))
        .map(|m| m.len())
        .unwrap_or(0);
    // A rewrite-the-whole-journal store would need tens of kilobytes per chunk at this size; a log needs a few hundred
    // bytes (redb pages included).
    assert!(size < chunks * 8 * 1024, "{size} bytes for {chunks} chunks");
}
