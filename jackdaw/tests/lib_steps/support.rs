//! The test clock, the task probe, fixtures and the world every library scenario runs in.
use chrono::{DateTime, TimeZone, Utc};
use jackdaw::api::{BoxFuture, Clock};
use jackdaw::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------------------------------
// Test clock
// ---------------------------------------------------------------------------------------------------------------

/// Virtual time that only moves when a step moves it, or when a step is waiting and nothing else can happen.
///
/// Every sleep registers its deadline, so a waiting step can jump straight to the next one.
#[derive(Clone)]
pub struct TestClock {
    inner: Arc<ClockInner>,
}

struct ClockInner {
    now: watch::Sender<DateTime<Utc>>,
    sleepers: Mutex<BTreeMap<u64, DateTime<Utc>>>,
    next_id: AtomicU64,
    start: DateTime<Utc>,
}

impl TestClock {
    /// How many sleeps have been started on this clock.
    pub fn sleeps_started(&self) -> u64 {
        self.inner.next_id.load(Ordering::SeqCst)
    }
}

impl fmt::Debug for TestClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestClock")
            .field("now", &self.now())
            .finish()
    }
}

impl TestClock {
    pub fn new() -> Self {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let (now, _) = watch::channel(start);
        Self {
            inner: Arc::new(ClockInner {
                now,
                sleepers: Mutex::new(BTreeMap::new()),
                next_id: AtomicU64::new(0),
                start,
            }),
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        *self.inner.now.borrow()
    }

    /// Virtual time since the scenario started.
    pub fn elapsed(&self) -> Duration {
        (self.now() - self.inner.start).to_std().unwrap_or_default()
    }

    pub fn advance(&self, by: Duration) {
        let to = self.now() + chrono::Duration::from_std(by).unwrap();
        self.advance_to(to);
    }

    pub fn advance_to(&self, to: DateTime<Utc>) {
        self.inner.now.send_modify(|now| {
            if to > *now {
                *now = to;
            }
        });
    }

    /// The earliest deadline anyone is sleeping until.
    pub fn next_deadline(&self) -> Option<DateTime<Utc>> {
        lock(&self.inner.sleepers).values().min().copied()
    }

    /// Jumps to the next deadline if it is within `horizon`. Returns whether it moved.
    pub fn advance_next(&self, horizon: Option<Duration>) -> bool {
        let Some(next) = self.next_deadline() else {
            return false;
        };
        if let Some(horizon) = horizon
            && next > self.now() + chrono::Duration::from_std(horizon).unwrap()
        {
            return false;
        }
        if next <= self.now() {
            return false;
        }
        self.advance_to(next);
        true
    }
}

struct Sleeper {
    clock: Arc<ClockInner>,
    id: u64,
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        lock(&self.clock.sleepers).remove(&self.id);
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        TestClock::now(self)
    }

    fn sleep_until(&self, deadline: DateTime<Utc>) -> BoxFuture<'static, ()> {
        let inner = Arc::clone(&self.inner);
        let id = inner.next_id.fetch_add(1, Ordering::SeqCst);
        lock(&inner.sleepers).insert(id, deadline);
        let guard = Sleeper {
            clock: Arc::clone(&inner),
            id,
        };
        let mut changes = inner.now.subscribe();
        Box::pin(async move {
            let _guard = guard;
            while *changes.borrow_and_update() < deadline {
                if changes.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The probe: scripted task behavior and a record of every invocation
// ---------------------------------------------------------------------------------------------------------------

/// A fault a scripted task fails with.
#[derive(Debug, Clone)]
pub struct FaultSpec {
    pub kind: String,
    pub status: Option<u16>,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub retryable: bool,
    /// Asks for this delay before the next attempt.
    pub next_retry_delay: Option<Duration>,
}

impl FaultSpec {
    pub fn non_retryable(detail: &str) -> Self {
        Self {
            kind: error_types::RUNTIME.to_string(),
            status: None,
            title: None,
            detail: Some(detail.to_string()),
            retryable: false,
            next_retry_delay: None,
        }
    }

    pub fn retryable(detail: &str) -> Self {
        Self {
            retryable: true,
            ..Self::non_retryable(detail)
        }
    }

    pub fn kind(kind: &str) -> Self {
        Self {
            kind: kind_uri(kind),
            detail: None,
            ..Self::non_retryable("")
        }
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    pub fn to_error(&self) -> TaskError {
        let mut error = TaskError::fault(&self.kind);
        if let Some(status) = self.status {
            error = error.status(status);
        }
        if let Some(title) = &self.title {
            error = error.title(title);
        }
        if let Some(detail) = &self.detail {
            error = error.detail(detail);
        }
        let error = if self.retryable { error.retry() } else { error };
        match self.next_retry_delay {
            Some(delay) => error.next_retry_delay(delay),
            None => error,
        }
    }
}

/// The error type URI for a short kind name ("validation", "payment", ...).
pub fn kind_uri(kind: &str) -> String {
    if kind.contains("://") {
        kind.to_string()
    } else {
        match kind {
            "validation" => error_types::VALIDATION.to_string(),
            "communication" => error_types::COMMUNICATION.to_string(),
            "runtime" => error_types::RUNTIME.to_string(),
            "timeout" => error_types::TIMEOUT.to_string(),
            other => other.to_string(),
        }
    }
}

/// How a task behaves in a scenario.
#[derive(Clone, Default)]
pub struct Script {
    /// The first `n` invocations fail with a retryable error.
    pub fail_first: u32,
    /// Attempt 1 of every call fails with a retryable error.
    pub fail_first_attempt: bool,
    /// Every invocation fails with this fault (after `fail_first`).
    pub fault: Option<FaultSpec>,
    /// Invocations whose input equals the key fail with the fault, at most `times` times (`None`: always).
    pub fail_on: Vec<(Value, FaultSpec, Option<u32>)>,
    pub panic: Option<String>,
    /// Takes this long on the test clock.
    pub delay: Option<Duration>,
    /// Sleeps the thread (for blocking tasks).
    pub thread_sleep: Option<Duration>,
    /// Waits for `Probe::release` before finishing.
    pub block: bool,
    /// Like `block`, but only for invocations with this input.
    pub block_on: Option<Value>,
    /// Returned in order, one per invocation; then `output`.
    pub outputs: VecDeque<Value>,
    pub output: Option<Value>,
    pub map: Option<Arc<dyn Fn(&Value) -> Value + Send + Sync>>,
}

/// One invocation of a task.
#[derive(Debug, Clone)]
pub struct Call {
    pub name: String,
    pub input: Value,
    pub attempt: u32,
    pub key: String,
    pub instance: String,
    pub started: u64,
    pub ended: Option<u64>,
    pub started_at: DateTime<Utc>,
    pub cancelled: bool,
}

/// Records and scripts task invocations. Shared by every engine a scenario builds, so counts survive a "crash".
pub struct Probe {
    pub clock: TestClock,
    scripts: Mutex<HashMap<String, Script>>,
    calls: Mutex<Vec<Call>>,
    counts: Mutex<HashMap<String, u32>>,
    in_flight: Mutex<HashMap<String, usize>>,
    max_in_flight: Mutex<HashMap<String, usize>>,
    total_in_flight: AtomicUsize,
    max_total_in_flight: AtomicUsize,
    gates: Mutex<HashMap<String, watch::Sender<bool>>>,
    tick: AtomicU64,
    pub defaults: Mutex<HashMap<String, Arc<dyn Fn(&Value) -> Value + Send + Sync>>>,
}

impl fmt::Debug for Probe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Probe").finish_non_exhaustive()
    }
}

/// Normalizes a task name from a scenario ("ReserveStock", "reserve_stock", "Inventory::reserve") to snake case.
pub fn task_key(name: &str) -> String {
    let name = name.rsplit("::").next().unwrap_or(name);
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else if ch == '-' || ch == ' ' {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    out
}

struct InFlight<'a> {
    probe: &'a Probe,
    name: String,
    index: usize,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let tick = self.probe.tick.fetch_add(1, Ordering::SeqCst);
        if let Some(count) = lock(&self.probe.in_flight).get_mut(&self.name) {
            *count = count.saturating_sub(1);
        }
        self.probe.total_in_flight.fetch_sub(1, Ordering::SeqCst);
        let mut calls = lock(&self.probe.calls);
        if let Some(call) = calls.get_mut(self.index)
            && call.ended.is_none()
        {
            // Not finished normally: dropped by a crash or a race, unless it panicked.
            call.ended = Some(tick);
            call.cancelled = !std::thread::panicking();
        }
    }
}

impl Probe {
    pub fn new(clock: TestClock) -> Arc<Self> {
        Arc::new(Self {
            clock,
            scripts: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
            counts: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            max_in_flight: Mutex::new(HashMap::new()),
            total_in_flight: AtomicUsize::new(0),
            max_total_in_flight: AtomicUsize::new(0),
            gates: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(0),
            defaults: Mutex::new(HashMap::new()),
        })
    }

    /// Changes how task `name` behaves.
    pub fn script(&self, name: &str, change: impl FnOnce(&mut Script)) {
        change(lock(&self.scripts).entry(task_key(name)).or_default());
    }

    /// Sets the output a task returns when no script says otherwise.
    pub fn default_output(&self, name: &str, f: impl Fn(&Value) -> Value + Send + Sync + 'static) {
        let previous = lock(&self.defaults).insert(task_key(name), Arc::new(f));
        assert!(
            previous.is_none(),
            "two harnesses set a default output for probe task `{name}`"
        );
    }

    fn gate(&self, name: &str) -> watch::Receiver<bool> {
        lock(&self.gates)
            .entry(task_key(name))
            .or_insert_with(|| watch::channel(false).0)
            .subscribe()
    }

    /// The fault task `name` is scripted to fail with.
    pub fn scripted_fault(&self, name: &str) -> Option<FaultSpec> {
        let scripts = lock(&self.scripts);
        let script = scripts.get(&task_key(name))?;
        script
            .fault
            .clone()
            .or_else(|| script.fail_on.first().map(|(_, fault, _)| fault.clone()))
    }

    /// Opens every gate: blocked tasks finish.
    pub fn release_all(&self) {
        for gate in lock(&self.gates).values() {
            gate.send_replace(true);
        }
        for script in lock(&self.scripts).values_mut() {
            script.block = false;
            script.block_on = None;
        }
    }

    /// Lets blocked invocations of `name` finish (and later ones pass straight through).
    pub fn release(&self, name: &str) {
        lock(&self.gates)
            .entry(task_key(name))
            .or_insert_with(|| watch::channel(false).0)
            .send_replace(true);
    }

    pub fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }

    pub fn calls_of(&self, name: &str) -> Vec<Call> {
        let key = task_key(name);
        self.calls().into_iter().filter(|c| c.name == key).collect()
    }

    pub fn count(&self, name: &str) -> usize {
        self.calls_of(name).len()
    }

    /// Invocations that finished (returned or failed), not counting ones cut short by a crash.
    pub fn completed(&self, name: &str) -> usize {
        self.calls_of(name)
            .iter()
            .filter(|c| c.ended.is_some() && !c.cancelled)
            .count()
    }

    pub fn max_in_flight(&self, name: &str) -> usize {
        lock(&self.max_in_flight)
            .get(&task_key(name))
            .copied()
            .unwrap_or(0)
    }

    pub fn max_total_in_flight(&self) -> usize {
        self.max_total_in_flight.load(Ordering::SeqCst)
    }

    pub fn running(&self, name: &str) -> bool {
        self.calls_of(name).iter().any(|c| c.ended.is_none())
    }

    /// Marks every unfinished invocation as cut short (the engine running it was dropped).
    pub fn abandon_running(&self) {
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        for call in lock(&self.calls).iter_mut() {
            if call.ended.is_none() {
                call.ended = Some(tick);
                call.cancelled = true;
            }
        }
        lock(&self.in_flight).clear();
        self.total_in_flight.store(0, Ordering::SeqCst);
    }

    /// Whether some invocation of `a` and some invocation of `b` were running at the same time.
    pub fn overlapped(&self, a: &str, b: &str) -> bool {
        let (left, right) = (self.calls_of(a), self.calls_of(b));
        left.iter().any(|x| {
            right.iter().any(|y| {
                let x_end = x.ended.unwrap_or(u64::MAX);
                let y_end = y.ended.unwrap_or(u64::MAX);
                x.started < y_end && y.started < x_end && !(a == b && x.started == y.started)
            })
        })
    }

    /// Runs task `name` according to its script.
    pub async fn invoke(
        &self,
        name: &str,
        ctx: &TaskContext,
        input: Value,
    ) -> Result<Value, TaskError> {
        self.invoke_inner(name, ctx, input).await.1
    }

    async fn invoke_inner(
        &self,
        name: &str,
        ctx: &TaskContext,
        input: Value,
    ) -> (usize, Result<Value, TaskError>) {
        let key = task_key(name);
        let n = {
            let mut counts = lock(&self.counts);
            let count = counts.entry(key.clone()).or_insert(0);
            *count += 1;
            *count
        };
        let index = {
            let tick = self.tick.fetch_add(1, Ordering::SeqCst);
            let mut calls = lock(&self.calls);
            calls.push(Call {
                name: key.clone(),
                input: input.clone(),
                attempt: ctx.attempt(),
                key: ctx.idempotency_key().to_string(),
                instance: ctx.instance_id().to_string(),
                started: tick,
                ended: None,
                started_at: self.clock.now(),
                cancelled: false,
            });
            calls.len() - 1
        };
        {
            let mut in_flight = lock(&self.in_flight);
            let now = in_flight.entry(key.clone()).or_insert(0);
            *now += 1;
            let mut max = lock(&self.max_in_flight);
            let peak = max.entry(key.clone()).or_insert(0);
            *peak = (*peak).max(*now);
        }
        let total = self.total_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_total_in_flight.fetch_max(total, Ordering::SeqCst);
        let _flight = InFlight {
            probe: self,
            name: key.clone(),
            index,
        };

        let result = self.scripted(&key, n, ctx, input).await;
        // Mark the end before the guard drops, so a normal end is not mistaken for a cut-short one.
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        if let Some(call) = lock(&self.calls).get_mut(index) {
            call.ended = Some(tick);
        }
        (index, result)
    }

    async fn scripted(
        &self,
        key: &str,
        n: u32,
        ctx: &TaskContext,
        input: Value,
    ) -> Result<Value, TaskError> {
        let key = key.to_string();
        let script = lock(&self.scripts).get(&key).cloned().unwrap_or_default();
        if let Some(delay) = script.delay {
            jackdaw::api::Clock::sleep_until(
                &self.clock,
                self.clock.now() + chrono::Duration::from_std(delay).unwrap(),
            )
            .await;
        }
        if let Some(sleep) = script.thread_sleep {
            std::thread::sleep(sleep);
        }
        if script.block || script.block_on.as_ref() == Some(&input) {
            let mut gate = self.gate(&key);
            while !*gate.borrow_and_update() {
                if gate.changed().await.is_err() {
                    break;
                }
            }
        }
        if let Some(message) = &script.panic {
            panic!("{message}");
        }
        if n <= script.fail_first {
            return Err(TaskError::retryable(format!(
                "{key} failed on invocation {n}"
            )));
        }
        if script.fail_first_attempt && ctx.attempt() == 1 {
            return Err(TaskError::retryable(format!("{key} failed on attempt 1")));
        }
        {
            let mut scripts = lock(&self.scripts);
            if let Some(live) = scripts.get_mut(&key) {
                for (when, fault, times) in &mut live.fail_on {
                    if *when == input && times.is_none_or(|left| left > 0) {
                        if let Some(left) = times {
                            *left -= 1;
                        }
                        return Err(fault.to_error());
                    }
                }
            }
        }
        if let Some(fault) = &script.fault {
            return Err(fault.to_error());
        }
        let queued = lock(&self.scripts)
            .get_mut(&key)
            .and_then(|live| live.outputs.pop_front());
        if let Some(output) = queued {
            return Ok(output);
        }
        if let Some(output) = script.output {
            return Ok(output);
        }
        if let Some(map) = script.map {
            return Ok(map(&input));
        }
        let default = lock(&self.defaults).get(&key).cloned();
        Ok(match default {
            Some(f) => f(&input),
            None => input,
        })
    }

    /// Runs task `name` with typed input and output.
    pub async fn typed<I: Serialize, O: DeserializeOwned>(
        &self,
        name: &str,
        ctx: &TaskContext,
        input: &I,
    ) -> Result<O, TaskError> {
        let value =
            serde_json::to_value(input).map_err(|e| TaskError::non_retryable(e.to_string()))?;
        let output = self.invoke(name, ctx, value).await?;
        serde_json::from_value(output.clone()).map_err(|e| {
            TaskError::non_retryable(format!(
                "probe output {output} for `{name}` has the wrong type: {e}"
            ))
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------------------------

/// What an awaited instance gave, with the error flattened so it can be cloned and inspected.
#[derive(Debug, Clone)]
pub enum Outcome {
    Ok(Value),
    Faulted(jackdaw::api::Fault),
    Timeout(String),
    Cancelled,
    Invalid(String),
    NonDeterminism(String),
    Other(String),
}

impl Outcome {
    pub fn of(result: Result<Value, Error>) -> Self {
        match result {
            Ok(value) => Self::Ok(value),
            Err(Error::Faulted(fault)) => Self::Faulted(fault),
            Err(Error::Timeout { detail }) => Self::Timeout(detail),
            Err(Error::Cancelled) => Self::Cancelled,
            Err(Error::Invalid(message)) => Self::Invalid(message),
            Err(Error::NonDeterminism(divergence)) => Self::NonDeterminism(divergence.to_string()),
            Err(other) => Self::Other(other.to_string()),
        }
    }

    pub fn value(&self) -> &Value {
        match self {
            Self::Ok(value) => value,
            other => panic!("expected the workflow to complete, but it ended with {other:?}"),
        }
    }

    pub fn fault(&self) -> &jackdaw::api::Fault {
        match self {
            Self::Faulted(fault) => fault,
            other => {
                panic!("expected the workflow to fail with a fault, but it ended with {other:?}")
            }
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Ok(value) => value.to_string(),
            Self::Faulted(fault) => fault.to_string(),
            Self::Timeout(detail) => format!("timeout: {detail}"),
            Self::Cancelled => "cancelled".to_string(),
            Self::Invalid(message) | Self::NonDeterminism(message) | Self::Other(message) => {
                message.clone()
            }
        }
    }
}

/// `json!` shorthand for tests.
pub fn v(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

pub fn null() -> Value {
    json!(null)
}

/// Polls `condition` until it holds, letting short timers (retry delays) fire. Panics after `limit` of real time.
pub async fn eventually(
    clock: &TestClock,
    limit: Duration,
    mut condition: impl FnMut() -> bool,
    what: &str,
) {
    let started = Instant::now();
    loop {
        if condition() {
            return;
        }
        if started.elapsed() > limit {
            panic!("timed out waiting until {what}");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        clock.advance_next(Some(Duration::from_secs(60)));
    }
}

/// Parses "3 hours", "40 minutes", "300 ms", "1 second", "30 days".
pub fn duration(text: &str) -> Duration {
    let text = text.trim();
    let (number, unit) = text
        .split_once(' ')
        .unwrap_or_else(|| panic!("a duration needs a number and a unit: `{text}`"));
    let n: f64 = number
        .parse()
        .unwrap_or_else(|_| panic!("bad duration `{text}`"));
    let seconds = match unit.trim_end_matches('s') {
        "m" | "millisecond" => n / 1000.0,
        "second" | "sec" => n,
        "minute" | "min" => n * 60.0,
        "hour" => n * 3600.0,
        "day" => n * 86400.0,
        "week" => n * 7.0 * 86400.0,
        other => panic!("unknown unit `{other}` in `{text}`"),
    };
    Duration::from_secs_f64(seconds)
}
