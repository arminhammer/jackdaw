//! The state of one scenario: its store, clock, probe, engine and the instances it started.
use super::support::{Outcome, Probe, TestClock, eventually};
use jackdaw::api::{BoxFuture, Recovery, WorkflowDef, WorkflowRef};
use jackdaw::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type Register = Arc<dyn Fn(EngineBuilder, &Harnesses) -> EngineBuilder + Send + Sync>;
type Start = Arc<
    dyn Fn(Engine, Value) -> BoxFuture<'static, Result<ExecutionHandle<Value>, Error>>
        + Send
        + Sync,
>;
type StartWithId = Arc<
    dyn Fn(Engine, String, Value) -> BoxFuture<'static, Result<ExecutionHandle<Value>, Error>>
        + Send
        + Sync,
>;
type Attach = Arc<
    dyn Fn(Engine, String) -> BoxFuture<'static, Result<ExecutionHandle<Value>, Error>>
        + Send
        + Sync,
>;

/// A workflow under test: how to register, start, resume and look it up, without knowing its types.
#[derive(Clone)]
pub struct Fixture {
    pub name: &'static str,
    /// The input "I execute it" uses.
    pub input: Value,
    pub register: Register,
    pub start: Start,
    pub start_with_id: StartWithId,
    pub resume: Attach,
    pub attach: Attach,
}

impl fmt::Debug for Fixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

impl Fixture {
    #[must_use]
    pub fn input(mut self, input: Value) -> Self {
        self.input = input;
        self
    }

    pub fn of<W>(name: &'static str) -> Self
    where
        W: WorkflowDef,
        W::In: serde::de::DeserializeOwned,
    {
        Self {
            name,
            input: Value::Null,
            register: Arc::new(|builder, _| builder.workflow::<W>()),
            start: Arc::new(|engine, input| {
                Box::pin(async move {
                    let typed: W::In = serde_json::from_value(input)
                        .map_err(|e| Error::invalid(format!("fixture input: {e}")))?;
                    Ok(engine
                        .execute(WorkflowRef::<W, W::In, W::Out>::new(), typed)
                        .await?
                        .erase())
                })
            }),
            start_with_id: Arc::new(|engine, id, input| {
                Box::pin(async move {
                    let typed: W::In = serde_json::from_value(input)
                        .map_err(|e| Error::invalid(format!("fixture input: {e}")))?;
                    Ok(engine
                        .execute_with_id(id, WorkflowRef::<W, W::In, W::Out>::new(), typed)
                        .await?
                        .erase())
                })
            }),
            resume: Arc::new(|engine, id| {
                Box::pin(async move { Ok(engine.resume::<W>(&id).await?.erase()) })
            }),
            attach: Arc::new(|engine, id| {
                Box::pin(async move { Ok(engine.handle::<W>(&id).await?.erase()) })
            }),
        }
    }
}

/// The task sets registered on every engine a scenario builds. They all share the scenario's probe.
pub struct Harnesses {
    pub probe: Arc<Probe>,
    pub sets: Vec<Box<dyn Fn(EngineBuilder) -> EngineBuilder + Send + Sync>>,
    pub inventory: Arc<super::calling_tasks::Inventory>,
}

impl Harnesses {
    pub fn inventory(&self) -> Arc<super::calling_tasks::Inventory> {
        Arc::clone(&self.inventory)
    }
}

/// One started instance.
pub struct Run {
    pub label: String,
    pub id: String,
    pub fixture: Option<Fixture>,
    pub handle: Option<ExecutionHandle<Value>>,
    pub outcome: Option<Outcome>,
    pub finished_at: Option<Instant>,
    pub started_virtual: chrono::DateTime<chrono::Utc>,
}

#[derive(cucumber::World)]
#[world(init = Self::new)]
pub struct LibWorld {
    pub dir: tempfile::TempDir,
    pub durable: bool,
    pub clock: TestClock,
    pub probe: Arc<Probe>,
    pub harnesses: Arc<Harnesses>,
    /// Workflows registered on every engine (all fixtures), unless `only` is set.
    pub defaults: Vec<Register>,
    /// When set, exactly these registrations (plus the task sets unless `no_tasks`).
    pub only: Option<Vec<Register>>,
    pub no_tasks: bool,
    pub engine: Option<Engine>,
    pub lifetimes: usize,
    pub fixture: Option<Fixture>,
    pub input: Option<Value>,
    pub runs: Vec<Run>,
    pub build_error: Option<String>,
    pub divergences: Option<Vec<jackdaw::api::Divergence>>,
    pub recovery: Option<Recovery>,
    pub notes: HashMap<String, Value>,
    pub mark: Option<usize>,
    /// A store the scenario built itself (it outlives engines, like a database would).
    pub persistence: Option<Arc<dyn jackdaw::persistence::PersistenceProvider>>,
    /// Extra builder settings for every engine of the scenario (a codec, ...).
    pub extras: Vec<Arc<dyn Fn(EngineBuilder) -> EngineBuilder + Send + Sync>>,
    /// The Postgres database of this scenario, when the store under test is Postgres.
    pub postgres_url: Option<String>,
    /// The durable store this scenario asks for (`redb`, `sqlite`, `postgres`), instead of `JACKDAW_TEST_STORE`.
    pub store: Option<String>,
    /// The workflow under test runs until cancelled, so "ran N times" checks must not wait for it to end.
    pub endless: bool,
    /// The recording/failing store of the durability scenarios.
    pub test_store: Option<Arc<super::durability::TestStore>>,
    /// The recording tracing subscriber of the observability scenarios.
    pub traces: Option<Arc<super::observability::Traces>>,
}

/// Which durable store the scenarios run on: `JACKDAW_TEST_STORE=redb|sqlite|postgres` (default redb). Postgres needs
/// `DATABASE_URL` pointing at a server where the test user may create databases.
pub fn store_kind() -> String {
    std::env::var("JACKDAW_TEST_STORE").unwrap_or_else(|_| "redb".to_string())
}

impl fmt::Debug for LibWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibWorld")
            .field("durable", &self.durable)
            .field("fixture", &self.fixture)
            .field(
                "runs",
                &self.runs.iter().map(|r| &r.label).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl LibWorld {
    async fn new() -> Self {
        let clock = TestClock::new();
        let probe = Probe::new(clock.clone());
        let harnesses = Arc::new(super::harnesses(&probe));
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
            durable: false,
            clock,
            probe,
            harnesses,
            defaults: super::all_fixtures(),
            only: None,
            no_tasks: false,
            engine: None,
            lifetimes: 0,
            fixture: None,
            input: None,
            runs: Vec::new(),
            build_error: None,
            divergences: None,
            recovery: None,
            notes: HashMap::new(),
            mark: None,
            persistence: None,
            extras: Vec::new(),
            postgres_url: None,
            endless: false,
            store: None,
            test_store: None,
            traces: None,
        }
    }

    /// The store this scenario runs on.
    pub fn store_kind(&self) -> String {
        self.store.clone().unwrap_or_else(store_kind)
    }

    /// A builder with this scenario's store, clock, tasks and workflows.
    pub fn builder(&self) -> EngineBuilder {
        let mut builder = Engine::builder().clock(self.clock.clone());
        if let Some(persistence) = &self.persistence {
            builder = builder.persistence(Arc::clone(persistence));
        } else if self.durable {
            builder = match self.store_kind().as_str() {
                "sqlite" => {
                    let path = self.dir.path().join("jackdaw.sqlite");
                    let store = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(jackdaw::api::Sqlite::open(&path))
                    })
                    .expect("the scenario's SQLite store opens");
                    builder.store(store)
                }
                "postgres" => {
                    let url = self
                        .postgres_url
                        .clone()
                        .expect("a Postgres database for the scenario");
                    let store = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(jackdaw::api::Postgres::connect(&url))
                    })
                    .expect("the scenario's Postgres store connects");
                    builder.store(store)
                }
                _ => builder.store(
                    Redb::open(self.dir.path().join("jackdaw.redb"))
                        .expect("the scenario's store opens"),
                ),
            };
        }
        for extra in &self.extras {
            builder = extra(builder);
        }
        if !self.no_tasks {
            for set in &self.harnesses.sets {
                builder = set(builder);
            }
        }
        let registrations = self.only.as_ref().unwrap_or(&self.defaults);
        for register in registrations {
            builder = register(builder, &self.harnesses);
        }
        builder
    }

    /// The engine, built on first use.
    pub fn engine(&mut self) -> Engine {
        if self.engine.is_none() {
            let engine = self
                .builder()
                .build()
                .expect("the scenario's engine builds");
            self.lifetimes += 1;
            self.engine = Some(engine);
        }
        self.engine.clone().expect("an engine")
    }

    pub fn fixture(&self) -> Fixture {
        self.fixture.clone().expect("the scenario names a workflow")
    }

    pub fn set_fixture(&mut self, fixture: Fixture) {
        self.fixture = Some(fixture);
    }

    /// Starts the current fixture with `input` under `label`.
    pub async fn start(&mut self, label: &str, input: Value) -> usize {
        let fixture = self.fixture();
        self.start_fixture(label, fixture, input).await
    }

    pub async fn start_fixture(&mut self, label: &str, fixture: Fixture, input: Value) -> usize {
        let engine = self.engine();
        let started_virtual = self.clock.now();
        let handle = (fixture.start)(engine, input)
            .await
            .unwrap_or_else(|e| panic!("could not start {}: {e}", fixture.name));
        self.push_run(label, fixture, handle, started_virtual)
    }

    pub fn push_run(
        &mut self,
        label: &str,
        fixture: Fixture,
        handle: ExecutionHandle<Value>,
        started_virtual: chrono::DateTime<chrono::Utc>,
    ) -> usize {
        self.runs.push(Run {
            label: label.to_string(),
            id: handle.id().to_string(),
            fixture: Some(fixture),
            handle: Some(handle),
            outcome: None,
            finished_at: None,
            started_virtual,
        });
        self.runs.len() - 1
    }

    pub fn run_index(&self, label: &str) -> usize {
        self.runs
            .iter()
            .rposition(|run| run.label == label)
            .unwrap_or_else(|| panic!("no run labelled `{label}`"))
    }

    /// The most recent run.
    pub fn last(&self) -> usize {
        self.runs
            .len()
            .checked_sub(1)
            .expect("the scenario started a workflow")
    }

    pub fn handle(&self, index: usize) -> ExecutionHandle<Value> {
        self.runs[index]
            .handle
            .clone()
            .expect("the run has a live handle (it was not lost in a crash)")
    }

    /// Waits for run `index` to end, moving the clock to the next deadline whenever nothing else is happening.
    pub async fn outcome(&mut self, index: usize) -> Outcome {
        self.outcome_within(index, Duration::from_secs(30)).await
    }

    /// Like [`outcome`](Self::outcome), with a different real-time limit (for long scenarios).
    pub async fn outcome_within(&mut self, index: usize, limit: Duration) -> Outcome {
        if let Some(outcome) = &self.runs[index].outcome {
            return outcome.clone();
        }
        let handle = self.handle(index);
        let mut waiting = Box::pin(handle.into_future());
        let started = Instant::now();
        // Move the clock only once nothing has happened for a few polls: a busy machine must not make the clock jump
        // past work that is merely slow.
        let (mut last_calls, mut idle) = (usize::MAX, 0);
        let outcome = loop {
            tokio::select! {
                result = &mut waiting => break Outcome::of(result),
                () = tokio::time::sleep(Duration::from_millis(15)) => {
                    let calls = self.probe.calls().len();
                    if calls == last_calls {
                        idle += 1;
                    } else {
                        idle = 0;
                        last_calls = calls;
                    }
                    if idle >= 3 {
                        self.clock.advance_next(None);
                        idle = 0;
                    }
                }
            }
            assert!(
                started.elapsed() < limit,
                "run `{}` did not finish within {limit:?} of real time",
                self.runs[index].label
            );
        };
        self.runs[index].outcome = Some(outcome.clone());
        self.runs[index].finished_at = Some(Instant::now());
        outcome
    }

    /// Waits for every run, in order. For a scenario about a workflow that never ends on its own (`endless`), lets it
    /// run without moving the clock instead.
    pub async fn settle(&mut self) {
        if self.endless {
            self.quiesce().await;
            return;
        }
        for index in 0..self.runs.len() {
            if self.runs[index].handle.is_some() {
                self.outcome(index).await;
            }
        }
    }

    /// Waits until a run has ended without moving the clock (for "has not run yet" checks).
    pub fn ended(&self, index: usize) -> bool {
        self.runs[index].outcome.is_some()
    }

    /// Simulates a crash: the engine and every handle to it are dropped, so its instances stop without ending.
    pub async fn crash(&mut self) {
        self.engine = None;
        for run in &mut self.runs {
            run.handle = None;
        }
        // Let the runtime drop the aborted instance tasks; with a durable store, until the file can be opened again.
        tokio::time::sleep(Duration::from_millis(20)).await;
        if self.durable && self.persistence.is_none() && self.store_kind() == "redb" {
            let path = self.dir.path().join("jackdaw.redb");
            let started = Instant::now();
            while Redb::open(&path).is_err() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "the dropped engine still holds the store after 5s"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        self.probe.abandon_running();
    }

    /// A new engine on the same store resumes run `index`.
    pub async fn resume(&mut self, index: usize) {
        let engine = self.engine();
        let fixture = self.runs[index]
            .fixture
            .clone()
            .expect("the run has a fixture");
        let id = self.runs[index].id.clone();
        let handle = (fixture.resume)(engine, id)
            .await
            .unwrap_or_else(|e| panic!("could not resume {}: {e}", fixture.name));
        self.runs[index].handle = Some(handle);
        self.runs[index].outcome = None;
    }

    /// A new engine on the same store recovers everything, and the runs get handles again.
    pub async fn recover(&mut self) {
        let engine = self.engine();
        let recovery = engine.recover().await.expect("recover");
        for index in 0..self.runs.len() {
            if self.runs[index].handle.is_none()
                && let Some(fixture) = self.runs[index].fixture.clone()
            {
                let id = self.runs[index].id.clone();
                if let Ok(handle) = (fixture.attach)(engine.clone(), id).await {
                    self.runs[index].handle = Some(handle);
                    self.runs[index].outcome = None;
                }
            }
        }
        self.recovery = Some(recovery);
    }

    /// Polls `condition` (letting short timers fire) for up to 10s of real time.
    pub async fn until(&self, what: &str, condition: impl FnMut() -> bool) {
        eventually(&self.clock, Duration::from_secs(10), condition, what).await;
    }

    /// Polls `condition` for up to 10s of real time without moving the clock.
    pub async fn until_frozen(&self, what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting until {what}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Lets the instances run until nothing changes for a moment, without moving the clock past long timers.
    pub async fn quiesce(&self) {
        let ceiling = self.clock.now() + chrono::Duration::seconds(60);
        self.quiesce_until(ceiling).await;
    }

    /// Like [`quiesce`](Self::quiesce), but never moves the clock past `ceiling`.
    pub async fn quiesce_until(&self, ceiling: chrono::DateTime<chrono::Utc>) {
        let mut last = usize::MAX;
        let mut stable = 0;
        while stable < 4 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if let Some(next) = self.clock.next_deadline()
                && next > self.clock.now()
                && next <= ceiling
            {
                self.clock.advance_to(next);
            }
            let now = self.probe.calls().len();
            if now == last {
                stable += 1;
            } else {
                stable = 0;
                last = now;
            }
        }
    }

    /// Moves virtual time forward in steps, letting each timer that comes due fire before the next.
    pub async fn pass(&self, by: Duration) {
        let target = self.clock.now() + chrono::Duration::from_std(by).expect("a duration");
        loop {
            self.quiesce_until(target).await;
            match self.clock.next_deadline() {
                Some(next) if next <= target && next > self.clock.now() => {
                    self.clock.advance_to(next)
                }
                _ => break,
            }
        }
        self.clock.advance_to(target);
        self.quiesce_until(target).await;
    }
}
