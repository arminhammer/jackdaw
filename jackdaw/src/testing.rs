//! Helpers for testing workflows.
//!
//! [`TestEngine`] is an [`Engine`] with an in-memory store that **replays every workflow from its own journal after
//! it finishes**, and fails the run with `Error::NonDeterminism` if the replay does not take the same steps or reach the
//! same result. It relies on tokio's paused clock, so use it from `#[tokio::test(start_paused = true)]`: sleeps and
//! timers then take no real time, and [`Clock::elapsed`] reports how much virtual time passed.
//!
//! Two clocks for tests that need control over time:
//!
//! - [`TokioClock`] follows tokio's (paused) clock, so `ctx.now()` agrees with virtual sleeps. [`TestEngine`] uses it.
//! - [`ManualClock`] only moves when the test calls [`ManualClock::advance`]. Share one between engines to keep time
//!   moving across a simulated crash.
use crate::api::{
    BoxFuture, CatalogDef, Clock as EngineClock, Engine, EngineBuilder, Error, IntoTaskSet,
    RegisterTask, Store, WorkflowDef,
};
use chrono::{DateTime, Utc};
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// A clock that follows tokio's clock: under `#[tokio::test(start_paused = true)]` it is virtual time.
///
/// `now()` is the wall time when the clock was created plus the tokio time that has passed since. Copies share the
/// same origin, so engines built with copies agree on the time.
#[derive(Debug, Clone, Copy)]
pub struct TokioClock {
    wall: DateTime<Utc>,
    instant: tokio::time::Instant,
}

impl TokioClock {
    /// Starts the clock at the current wall time.
    #[must_use]
    pub fn new() -> Self {
        Self::starting_at(Utc::now())
    }

    /// Starts the clock at `start`.
    #[must_use]
    pub fn starting_at(start: DateTime<Utc>) -> Self {
        Self {
            wall: start,
            instant: tokio::time::Instant::now(),
        }
    }
}

impl Default for TokioClock {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineClock for TokioClock {
    fn now(&self) -> DateTime<Utc> {
        let elapsed = tokio::time::Instant::now().saturating_duration_since(self.instant);
        crate::api::clock_after(self.wall, elapsed)
    }

    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'static, ()> {
        let offset = (deadline - self.wall).to_std().unwrap_or(Duration::ZERO);
        Box::pin(tokio::time::sleep_until(self.instant + offset))
    }
}

/// A clock that stands still until the test moves it. Clones share the same time.
///
/// ```
/// use jackdaw::prelude::*;
/// use jackdaw::testing::ManualClock;
///
/// let clock = ManualClock::new();
/// let start = clock.now();
/// clock.advance(2.hours());
/// assert_eq!(clock.now() - start, chrono::Duration::hours(2));
/// ```
#[derive(Debug, Clone)]
pub struct ManualClock {
    now: Arc<watch::Sender<DateTime<Utc>>>,
}

impl ManualClock {
    /// A clock that starts at the current wall time.
    #[must_use]
    pub fn new() -> Self {
        Self::starting_at(Utc::now())
    }

    /// A clock that starts at `start`.
    #[must_use]
    pub fn starting_at(start: DateTime<Utc>) -> Self {
        let (now, _) = watch::channel(start);
        Self { now: Arc::new(now) }
    }

    /// The clock's current time.
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        *self.now.borrow()
    }

    /// Moves time forward, waking every sleep whose deadline has passed.
    pub fn advance(&self, by: Duration) {
        self.now
            .send_modify(|now| *now = crate::api::clock_after(*now, by));
    }

    /// Sets the time. Moving it backwards does not wake anything.
    pub fn set(&self, to: DateTime<Utc>) {
        self.now.send_replace(to);
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        Self::now(self)
    }

    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'static, ()> {
        let mut changes = self.now.subscribe();
        Box::pin(async move {
            while *changes.borrow_and_update() < deadline {
                if changes.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        })
    }
}

/// Virtual time since the engine was built.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    started: tokio::time::Instant,
}

impl Clock {
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        tokio::time::Instant::now().saturating_duration_since(self.started)
    }
}

/// An [`Engine`] for tests. It derefs to the engine, so `execute`, `run_task` and the rest work unchanged.
#[derive(Debug, Clone)]
pub struct TestEngine {
    engine: Engine,
    clock: Clock,
}

impl TestEngine {
    #[must_use]
    pub fn builder() -> TestEngineBuilder {
        TestEngineBuilder {
            inner: EngineBuilder::new().self_replay(true),
            clock: None,
        }
    }

    #[must_use]
    pub fn clock(&self) -> Clock {
        self.clock
    }
}

impl Deref for TestEngine {
    type Target = Engine;

    fn deref(&self) -> &Engine {
        &self.engine
    }
}

/// Builds a [`TestEngine`]; it has the registration methods of [`EngineBuilder`].
pub struct TestEngineBuilder {
    inner: EngineBuilder,
    clock: Option<Box<dyn FnOnce(EngineBuilder) -> EngineBuilder + Send>>,
}

impl TestEngineBuilder {
    #[must_use]
    pub fn task(mut self, task: impl RegisterTask) -> Self {
        self.inner = self.inner.task(task);
        self
    }

    #[must_use]
    pub fn tasks(mut self, set: impl IntoTaskSet) -> Self {
        self.inner = self.inner.tasks(set);
        self
    }

    #[must_use]
    pub fn workflow<W: WorkflowDef>(mut self) -> Self {
        self.inner = self.inner.workflow::<W>();
        self
    }

    #[must_use]
    pub fn catalog<C: CatalogDef>(mut self) -> Self {
        self.inner = self.inner.catalog::<C>();
        self
    }

    #[must_use]
    pub fn store(mut self, store: impl Store) -> Self {
        self.inner = self.inner.store(store);
        self
    }

    /// Uses `clock` instead of a fresh [`TokioClock`].
    #[must_use]
    pub fn clock(mut self, clock: impl EngineClock) -> Self {
        self.clock = Some(Box::new(move |builder| builder.clock(clock)));
        self
    }

    /// Switches off the replay check (it is on by default).
    #[must_use]
    pub fn self_replay(mut self, enabled: bool) -> Self {
        self.inner = self.inner.self_replay(enabled);
        self
    }

    /// # Errors
    /// The same as [`EngineBuilder::build`].
    pub fn build(self) -> Result<TestEngine, Error> {
        let inner = match self.clock {
            Some(apply) => apply(self.inner),
            None => self.inner.clock(TokioClock::new()),
        };
        Ok(TestEngine {
            engine: inner.build()?,
            clock: Clock {
                started: tokio::time::Instant::now(),
            },
        })
    }
}
