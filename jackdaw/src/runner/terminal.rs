use super::lines::{Describe, indent_json};
use super::{Report, Reporter};
use crate::api::Event;
use chrono::{DateTime, Utc};
use console::style;
use indicatif::{HumanDuration, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::time::Instant;

/// A progress bar on standard error that names the steps in flight, a line for each step that finishes, fails, is
/// tried again or was done before a resume, and a summary at the end. Where standard error is not a terminal nothing is
/// drawn but the lines.
///
/// Several runs can share one display: make the first with [`new`](Self::new) or
/// [`with_target`](Self::with_target), and the others with [`in_group`](Self::in_group) on its
/// [`group`](Self::group), each [`label`](Self::label)led.
pub struct Terminal {
    multi: MultiProgress,
    bar: ProgressBar,
    label: Option<String>,
    describe: Option<Describe>,
    total: Option<usize>,
    outputs: bool,
    running: BTreeSet<String>,
    started: HashMap<String, VecDeque<DateTime<Utc>>>,
    done: usize,
    failed: usize,
    retried: usize,
    replayed: usize,
    began: Instant,
}

impl Terminal {
    /// Draws to standard error.
    #[must_use]
    pub fn new() -> Self {
        Self::with_target(ProgressDrawTarget::stderr())
    }

    /// Draws to `target`.
    #[must_use]
    pub fn with_target(target: ProgressDrawTarget) -> Self {
        Self::in_group(&MultiProgress::with_draw_target(target))
    }

    /// Draws into a display other terminals share, so runs side by side keep their bars apart.
    #[must_use]
    pub fn in_group(group: &MultiProgress) -> Self {
        let bar = group.add(ProgressBar::new_spinner());
        bar.set_style(spinner());
        Self {
            multi: group.clone(),
            bar,
            label: None,
            describe: None,
            total: None,
            outputs: false,
            running: BTreeSet::new(),
            started: HashMap::new(),
            done: 0,
            failed: 0,
            retried: 0,
            replayed: 0,
            began: Instant::now(),
        }
    }

    /// The display this terminal draws into, to give to [`in_group`](Self::in_group).
    #[must_use]
    pub fn group(&self) -> MultiProgress {
        self.multi.clone()
    }

    /// Starts each line with `[label]`: for telling runs apart in a shared display.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Adds `describe`'s text to the line of each step that finishes. See [`Lines::describe`](super::Lines::describe).
    #[must_use]
    pub fn describe(
        mut self,
        describe: impl Fn(&str, &Value) -> Option<String> + Send + 'static,
    ) -> Self {
        self.describe = Some(Box::new(describe));
        self
    }

    /// The number of steps the run is expected to have: the spinner becomes a bar.
    #[must_use]
    pub fn total(mut self, steps: usize) -> Self {
        self.total = Some(steps);
        self.bar.set_length(steps as u64);
        self.bar.set_style(bar());
        self
    }

    /// Shows what each finished step returned, as indented JSON under its line.
    #[must_use]
    pub fn outputs(mut self, show: bool) -> Self {
        self.outputs = show;
        self
    }

    fn line(&self, text: &str) {
        let text = match &self.label {
            Some(label) => text
                .lines()
                .map(|line| format!("{} {line}", style(format!("[{label}]")).dim()))
                .collect::<Vec<_>>()
                .join("\n"),
            None => text.to_string(),
        };
        // A hidden target drops the line, which is what a pipe wants.
        let _ = self.multi.println(text);
    }

    fn show_running(&self) {
        let names: Vec<&str> = self.running.iter().map(String::as_str).take(3).collect();
        let more = self.running.len().saturating_sub(names.len());
        let tail = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        let label = self
            .label
            .as_ref()
            .map_or_else(String::new, |label| format!("[{label}] "));
        self.bar
            .set_message(format!("{label}{}{tail}", names.join(", ")));
    }

    /// How long the step that just ended took, from the events' own timestamps.
    fn took(&mut self, task: &str, at: DateTime<Utc>) -> String {
        self.started
            .get_mut(task)
            .and_then(VecDeque::pop_front)
            .map_or_else(String::new, |began| {
                format!(" ({})", duration((at - began).num_milliseconds()))
            })
    }

    // `Event` can grow: what a reporter does not know it leaves alone.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn event(&mut self, event: &Event) {
        match event {
            Event::TaskStarted {
                task_name,
                replayed: false,
                timestamp,
                ..
            } => {
                self.started
                    .entry(task_name.clone())
                    .or_default()
                    .push_back(*timestamp);
                self.running.insert(task_name.clone());
                self.show_running();
            }
            Event::TaskCompleted {
                task_name,
                replayed: true,
                ..
            } => {
                self.replayed += 1;
                self.done += 1;
                self.bar.inc(1);
                self.line(&format!(
                    "{} {task_name} (already completed)",
                    style("⤼").yellow()
                ));
            }
            Event::TaskCompleted {
                task_name,
                result,
                timestamp,
                ..
            } => {
                self.running.remove(task_name);
                self.show_running();
                self.done += 1;
                self.bar.inc(1);
                let took = self.took(task_name, *timestamp);
                let text = self
                    .describe
                    .as_ref()
                    .and_then(|describe| describe(task_name, result))
                    .map_or_else(|| task_name.clone(), |text| format!("{task_name}: {text}"));
                let mut line = format!("{} {text}{took}", style("✔").green());
                if self.outputs {
                    line.push('\n');
                    line.push_str(&style(indent_json(result, 4)).green().to_string());
                }
                self.line(&line);
            }
            Event::TaskFaulted {
                task_name,
                error,
                timestamp,
                ..
            } => {
                self.running.remove(task_name);
                self.show_running();
                self.failed += 1;
                let took = self.took(task_name, *timestamp);
                self.line(&format!("{} {task_name}: {error}{took}", style("✘").red()));
            }
            Event::TaskRetried {
                task_name, attempt, ..
            } => {
                self.retried += 1;
                self.line(&format!(
                    "{} {task_name} retrying after attempt {attempt}",
                    style("↻").yellow()
                ));
            }
            _ => {}
        }
    }

    fn summary(&self) -> String {
        let mut text = format!("{} steps done, {} failed", self.done, self.failed);
        if self.retried > 0 {
            text.push_str(&format!(", {} retried", self.retried));
        }
        if self.replayed > 0 {
            text.push_str(&format!(", {} replayed", self.replayed));
        }
        format!("{text} in {}", HumanDuration(self.began.elapsed()))
    }

    fn begin(&mut self) {
        self.began = Instant::now();
        self.bar
            .enable_steady_tick(std::time::Duration::from_millis(120));
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

/// `1.50s` or `35ms`, the way a step's duration reads.
fn duration(ms: i64) -> String {
    #[allow(clippy::cast_precision_loss)]
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.2}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {:.2}s", ms / 60_000, (ms % 60_000) as f64 / 1000.0)
    }
}

fn spinner() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg} {elapsed}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
}

fn bar() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} [{bar:30.cyan/blue}] {pos}/{len} {msg} {elapsed}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("#>-")
}

impl Reporter for Terminal {
    fn report(&mut self, report: Report<'_>) {
        match report {
            Report::Starting { id } => {
                self.begin();
                self.line(&format!("{} {id}", style("▸ starting").cyan()));
            }
            Report::Resuming { id } => {
                self.begin();
                self.line(&format!("{} {id}", style("↻ resuming").yellow().bold()));
            }
            Report::Restarting { id } => {
                self.line(&format!("{} {id}", style("↻ restarting").yellow()))
            }
            Report::Finished { id } => {
                self.line(&format!("{id} has finished; its stored result follows"));
            }
            Report::Event(event) => self.event(event),
            Report::Done => {
                self.bar.finish_and_clear();
                let summary = self.summary();
                self.line(&summary);
            }
        }
    }
}
