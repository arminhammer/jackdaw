//! Schedules: start a workflow on a cron expression or at a fixed interval.
//!
//! ```no_run
//! use jackdaw::api::{CatchUp, Overlap, Schedule};
//! use jackdaw::prelude::*;
//!
//! #[workflow(namespace = "ops", name = "report", version = "1.0.0")]
//! struct Report;
//!
//! #[workflow_methods]
//! impl Report {
//!     #[run]
//!     async fn run(_ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
//!         Ok(())
//!     }
//! }
//!
//! # fn main() -> Result<(), Error> {
//! let engine = Engine::builder()
//!     .workflow::<Report>()
//!     .schedule(Schedule::cron("nightly-report", "0 2 * * *", Report::run, ())?.catch_up(CatchUp::One))
//!     .build()?;
//! # let _ = engine;
//! # Ok(())
//! # }
//! ```
//!
//! Each fire starts an instance with the id `<schedule>/<fire time>` (for example
//! `nightly-report/2026-01-01T02:00:00Z`), so engines that share a store never start the same tick twice. When a fire
//! time was reached is stored, so a restart neither repeats a tick nor forgets one: ticks that passed while no engine
//! ran are handled by the schedule's [`CatchUp`] policy.
use super::engine::{Engine, ExecuteOptions};
use super::error::Error;
use super::task::BoxFuture;
use super::workflow::Runnable;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, SecondsFormat, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------------------------------------------
// Cron expressions
// ---------------------------------------------------------------------------------------------------------------

/// A five-field cron expression: minute, hour, day of month, month, day of week (UTC).
///
/// Each field takes `*`, numbers, ranges (`1-5`), steps (`*/15`, `0-30/10`) and lists (`1,15`). Day of week is 0-7
/// with both 0 and 7 meaning Sunday. As in classic cron, when both day fields are restricted a day matches either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: Vec<bool>,
    hours: Vec<bool>,
    days: Vec<bool>,
    months: Vec<bool>,
    weekdays: Vec<bool>,
    days_restricted: bool,
    weekdays_restricted: bool,
}

fn parse_field(field: &str, min: u32, max: u32, what: &str) -> Result<(Vec<bool>, bool), Error> {
    let mut allowed = vec![false; usize::try_from(max + 1).unwrap_or(0)];
    let invalid = || Error::invalid(format!("invalid cron {what} field `{field}`"));
    for part in field.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().map_err(|_| invalid())?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(invalid());
        }
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse().map_err(|_| invalid())?,
                b.parse().map_err(|_| invalid())?,
            )
        } else {
            let value: u32 = range.parse().map_err(|_| invalid())?;
            (value, if part.contains('/') { max } else { value })
        };
        if start < min || end > max || start > end {
            return Err(invalid());
        }
        let mut value = start;
        while value <= end {
            if let Some(slot) = allowed.get_mut(usize::try_from(value).unwrap_or(usize::MAX)) {
                *slot = true;
            }
            value += step;
        }
    }
    Ok((allowed, field != "*"))
}

impl Cron {
    /// Parses `expression`.
    ///
    /// # Errors
    /// `Error::Invalid` naming the field that does not parse.
    pub fn parse(expression: &str) -> Result<Self, Error> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(Error::invalid(format!(
                "a cron expression has five fields (minute hour day month weekday), not `{expression}`"
            )));
        };
        let (minutes, _) = parse_field(minute, 0, 59, "minute")?;
        let (hours, _) = parse_field(hour, 0, 23, "hour")?;
        let (days, days_restricted) = parse_field(day, 1, 31, "day-of-month")?;
        let (months, _) = parse_field(month, 1, 12, "month")?;
        let (mut weekdays, weekdays_restricted) = parse_field(weekday, 0, 7, "day-of-week")?;
        if weekdays.get(7).copied().unwrap_or(false)
            && let Some(sunday) = weekdays.get_mut(0)
        {
            *sunday = true;
        }
        Ok(Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            days_restricted,
            weekdays_restricted,
        })
    }

    fn day_matches(&self, at: DateTime<Utc>) -> bool {
        let allowed = |set: &[bool], value: u32| set.get(value as usize).copied().unwrap_or(false);
        let month = allowed(&self.months, at.month());
        let day = allowed(&self.days, at.day());
        let weekday = allowed(&self.weekdays, at.weekday().num_days_from_sunday());
        let day_ok = match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => day || weekday,
            (true, false) => day,
            (false, true) => weekday,
            (false, false) => true,
        };
        month && day_ok
    }

    /// The first time strictly after `after` that matches.
    #[must_use]
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let allowed = |set: &[bool], value: u32| set.get(value as usize).copied().unwrap_or(false);
        let mut at = after.with_second(0)?.with_nanosecond(0)? + ChronoDuration::minutes(1);
        let limit = after + ChronoDuration::days(366 * 5);
        while at <= limit {
            if !self.day_matches(at) {
                at = (at + ChronoDuration::days(1))
                    .with_hour(0)?
                    .with_minute(0)?;
                continue;
            }
            if !allowed(&self.hours, at.hour()) {
                at = (at + ChronoDuration::hours(1)).with_minute(0)?;
                continue;
            }
            if !allowed(&self.minutes, at.minute()) {
                at += ChronoDuration::minutes(1);
                continue;
            }
            return Some(at);
        }
        None
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------------------------------------------

/// What to do with fire times that passed while no engine was running the schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CatchUp {
    /// Fire none of them; continue with the next one. The default.
    #[default]
    Skip,
    /// Fire the latest one.
    One,
    /// Fire each of them, oldest first.
    All,
}

/// What to do when a fire time comes while the previous instance of the schedule is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Overlap {
    /// Start the new instance anyway. The default.
    #[default]
    Allow,
    /// Skip this fire time.
    Skip,
}

#[derive(Debug, Clone)]
enum When {
    Cron(Cron),
    Every(Duration),
}

impl When {
    fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Cron(cron) => cron.next_after(after),
            Self::Every(interval) => Some(super::clock::after(after, *interval)),
        }
    }
}

type Start = Arc<dyn Fn(Engine, String) -> BoxFuture<'static, Result<(), Error>> + Send + Sync>;

/// A workflow to start on a schedule. Register it with [`EngineBuilder::schedule`](super::EngineBuilder::schedule).
#[derive(Clone)]
pub struct Schedule {
    pub(crate) name: String,
    when: When,
    start: Start,
    catch_up: CatchUp,
    overlap: Overlap,
}

impl std::fmt::Debug for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Schedule")
            .field("name", &self.name)
            .field("when", &self.when)
            .field("catch_up", &self.catch_up)
            .field("overlap", &self.overlap)
            .finish_non_exhaustive()
    }
}

impl Schedule {
    /// Starts `workflow` with `input` at every time `cron` matches (UTC).
    ///
    /// # Errors
    /// `Error::Invalid` if the cron expression does not parse.
    pub fn cron<R>(
        name: impl Into<String>,
        cron: &str,
        workflow: R,
        input: R::In,
    ) -> Result<Self, Error>
    where
        R: Runnable + Clone + Send + Sync + 'static,
        R::In: Clone + Sync,
    {
        Ok(Self::new(
            name,
            When::Cron(Cron::parse(cron)?),
            workflow,
            input,
        ))
    }

    /// Starts `workflow` with `input` every `interval`, counted from when the schedule first runs.
    #[must_use]
    pub fn every<R>(name: impl Into<String>, interval: Duration, workflow: R, input: R::In) -> Self
    where
        R: Runnable + Clone + Send + Sync + 'static,
        R::In: Clone + Sync,
    {
        Self::new(name, When::Every(interval), workflow, input)
    }

    fn new<R>(name: impl Into<String>, when: When, workflow: R, input: R::In) -> Self
    where
        R: Runnable + Clone + Send + Sync + 'static,
        R::In: Clone + Sync,
    {
        let start: Start = Arc::new(move |engine: Engine, id: String| {
            let (workflow, input) = (workflow.clone(), input.clone());
            Box::pin(async move {
                engine
                    .execute_with(workflow, input, ExecuteOptions::new().id(id).create_only())
                    .await
                    .map(|_| ())
            })
        });
        Self {
            name: name.into(),
            when,
            start,
            catch_up: CatchUp::Skip,
            overlap: Overlap::Allow,
        }
    }

    /// What to do with fire times that passed while no engine ran.
    #[must_use]
    pub fn catch_up(mut self, policy: CatchUp) -> Self {
        self.catch_up = policy;
        self
    }

    /// What to do when the previous instance is still running.
    #[must_use]
    pub fn overlap(mut self, policy: Overlap) -> Self {
        self.overlap = policy;
        self
    }

    /// The instance id of the fire at `at`.
    #[must_use]
    pub fn instance_id(&self, at: DateTime<Utc>) -> String {
        format!(
            "{}/{}",
            self.name,
            at.to_rfc3339_opts(SecondsFormat::Secs, true)
        )
    }
}

/// The durable part of a schedule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ScheduleState {
    /// The last fire time that was handled (fired or skipped).
    pub last: Option<DateTime<Utc>>,
    pub paused: bool,
    pub removed: bool,
    /// The instance started by the latest fire.
    pub latest: Option<String>,
}

pub(crate) fn state_id(name: &str) -> String {
    format!("__jackdaw_schedule__/{name}")
}

async fn load_state(engine: &Engine, name: &str) -> Result<ScheduleState, Error> {
    let checkpoint = engine
        .shared()
        .persistence
        .get_checkpoint(&state_id(name))
        .await?;
    Ok(checkpoint
        .and_then(|checkpoint| {
            serde_json::from_value(checkpoint.data.get("schedule")?.clone()).ok()
        })
        .unwrap_or_default())
}

pub(crate) async fn save_state(
    engine: &Engine,
    name: &str,
    state: &ScheduleState,
) -> Result<(), Error> {
    engine
        .shared()
        .persistence
        .save_checkpoint(crate::workflow::WorkflowCheckpoint {
            instance_id: state_id(name),
            current_task: String::new(),
            data: json!({ "schedule": state }),
            timestamp: Utc::now(),
        })
        .await?;
    Ok(())
}

pub(crate) async fn update_state(
    engine: &Engine,
    name: &str,
    change: impl FnOnce(&mut ScheduleState),
) -> Result<(), Error> {
    let mut state = load_state(engine, name).await?;
    change(&mut state);
    save_state(engine, name, &state).await?;
    engine.shared().schedules_changed.notify_waiters();
    Ok(())
}

impl Schedule {
    async fn fire(
        &self,
        engine: &Engine,
        at: DateTime<Utc>,
        state: &mut ScheduleState,
    ) -> Result<(), Error> {
        let skip = self.overlap == Overlap::Skip
            && match &state.latest {
                Some(latest) => matches!(engine.status(latest).await, Ok(super::Status::Running)),
                None => false,
            };
        if !skip {
            let id = self.instance_id(at);
            match (self.start)(engine.clone(), id.clone()).await {
                // The instance exists: another engine on the same store (or this one, before a restart) fired this
                // tick. It is never attached to, since that would resume it next to the engine already running it.
                Ok(()) | Err(Error::Invalid(_)) => state.latest = Some(id),
                Err(error) => return Err(error),
            }
        }
        state.last = Some(at);
        save_state(engine, &self.name, state).await
    }

    /// Runs the schedule until it is removed or the engine is dropped.
    pub(crate) async fn run(
        self,
        shared: std::sync::Weak<super::runtime::Shared>,
    ) -> Result<(), Error> {
        let mut starting = true;
        loop {
            let engine = Engine::from_shared(
                shared
                    .upgrade()
                    .ok_or_else(|| Error::Engine("dropped".into()))?,
            );
            let clock = Arc::clone(&engine.shared().clock);
            let changed = Arc::clone(&engine.shared().schedules_changed);
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let mut state = load_state(&engine, &self.name).await?;
            if state.removed {
                return Ok(());
            }
            if state.paused {
                drop(engine);
                notified.await;
                starting = true;
                continue;
            }
            let now = clock.now();
            let Some(last) = state.last else {
                // First run: fire times count from now.
                state.last = Some(now);
                save_state(&engine, &self.name, &state).await?;
                continue;
            };
            if starting {
                starting = false;
                // Fire times that passed while no engine ran this schedule.
                let mut missed = Vec::new();
                let mut at = last;
                while let Some(next) = self.when.next_after(at) {
                    if next > now || missed.len() >= 10_000 {
                        break;
                    }
                    missed.push(next);
                    at = next;
                }
                if let Some(&latest) = missed.last() {
                    match self.catch_up {
                        CatchUp::Skip => {
                            state.last = Some(latest);
                            save_state(&engine, &self.name, &state).await?;
                        }
                        CatchUp::One => self.fire(&engine, latest, &mut state).await?,
                        CatchUp::All => {
                            for at in missed {
                                self.fire(&engine, at, &mut state).await?;
                            }
                        }
                    }
                    continue;
                }
            }
            let Some(next) = self.when.next_after(last) else {
                return Ok(());
            };
            if next <= now {
                self.fire(&engine, next, &mut state).await?;
                continue;
            }
            drop(engine);
            tokio::select! {
                () = clock.sleep_until(next) => {}
                () = &mut notified => {}
            }
        }
    }
}

/// Runs a schedule and logs (instead of losing silently) why it stopped.
pub(crate) async fn drive(schedule: Schedule, shared: std::sync::Weak<super::runtime::Shared>) {
    let name = schedule.name.clone();
    if let Err(error) = schedule.run(shared).await {
        tracing::error!(schedule = %name, %error, "a schedule stopped");
    }
}
