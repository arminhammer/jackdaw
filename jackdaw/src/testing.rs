//! Helpers for testing workflows.
//!
//! [`TestEngine`] is an [`Engine`] with an in-memory store that **replays every workflow from its own journal after
//! it finishes**, and fails the run with `Error::NonDeterminism` if the replay does not take the same steps or reach the
//! same result. It relies on tokio's paused clock, so use it from `#[tokio::test(start_paused = true)]`: sleeps and
//! timers then take no real time, and [`Clock::elapsed`] reports how much virtual time passed.
use crate::api::{
    CatalogDef, Engine, EngineBuilder, Error, IntoTaskSet, Redb, RegisterTask, WorkflowDef,
};
use std::ops::Deref;
use std::time::Duration;

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
    pub fn store(mut self, store: Redb) -> Self {
        self.inner = self.inner.store(store);
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
        Ok(TestEngine {
            engine: self.inner.build()?,
            clock: Clock {
                started: tokio::time::Instant::now(),
            },
        })
    }
}
