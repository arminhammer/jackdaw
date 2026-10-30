//! Where the engine gets the time from.
//!
//! Everything time-related in a workflow goes through a [`Clock`]: [`WorkflowContext::now`](super::WorkflowContext::now),
//! durable sleeps, step timeouts and retry delays. Deadlines are stored in the journal as wall-clock times, so a
//! workflow that is resumed by a new process waits only for the time that is left.
//!
//! Production code uses [`SystemClock`] (the default). Tests use the clocks in [`crate::testing`], which can be
//! shared between engines so that "time" keeps moving across a simulated crash.
use super::task::BoxFuture;
use chrono::{DateTime, Utc};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// A source of time for the engine.
pub trait Clock: Send + Sync + fmt::Debug + 'static {
    /// The current time.
    fn now(&self) -> DateTime<Utc>;

    /// Resolves once [`now`](Self::now) has reached `deadline`. Resolves immediately for a deadline in the past.
    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'static, ()>;
}

/// The wall clock, with tokio timers for waiting.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'static, ()> {
        let wait = remaining(Utc::now(), deadline);
        Box::pin(tokio::time::sleep(wait))
    }
}

/// How long from `now` until `deadline`, zero if it has passed.
pub(crate) fn remaining(now: DateTime<Utc>, deadline: DateTime<Utc>) -> Duration {
    (deadline - now).to_std().unwrap_or(Duration::ZERO)
}

/// `now + duration`, saturating instead of overflowing.
#[doc(hidden)]
#[must_use]
pub fn after(now: DateTime<Utc>, duration: Duration) -> DateTime<Utc> {
    chrono::Duration::from_std(duration)
        .ok()
        .and_then(|delta| now.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// A shared clock.
pub(crate) type SharedClock = Arc<dyn Clock>;

/// Sleeps for `duration` on `clock`.
pub(crate) fn sleep(clock: &dyn Clock, duration: Duration) -> BoxFuture<'static, ()> {
    clock.sleep_until(after(clock.now(), duration))
}
