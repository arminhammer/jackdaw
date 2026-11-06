use super::{Report, Reporter};
use crate::api::Event;
use serde_json::{Value, json};
use std::io::Write;

/// Adds to a step's line: given the task's name and what it returned, the text to show, or `None` for the plain line.
pub(super) type Describe = Box<dyn Fn(&str, &Value) -> Option<String> + Send>;

/// One line per step: when it starts, when it finishes, when it fails or is tried again. For logs, CI, and anything
/// that is not a terminal.
///
/// ```text
/// starting pipeline/today
/// fetch started
/// fetch done [1/2]
/// merge started
/// merge done [2/2]
/// pipeline/today completed
/// ```
pub struct Lines<W: Write + Send = std::io::Stderr> {
    out: W,
    describe: Option<Describe>,
    total: Option<usize>,
    outputs: bool,
    done: usize,
}

impl Lines {
    /// Lines on standard error.
    #[must_use]
    pub fn stderr() -> Self {
        Self::new(std::io::stderr())
    }
}

impl<W: Write + Send> Lines<W> {
    /// Lines written to `out`.
    #[must_use]
    pub fn new(out: W) -> Self {
        Self {
            out,
            describe: None,
            total: None,
            outputs: false,
            done: 0,
        }
    }

    /// Adds `describe`'s text to the line of each step that finishes, as `name done: text`. It gets the task's name and
    /// its output, and may return `None` to leave a step's line plain.
    #[must_use]
    pub fn describe(
        mut self,
        describe: impl Fn(&str, &Value) -> Option<String> + Send + 'static,
    ) -> Self {
        self.describe = Some(Box::new(describe));
        self
    }

    /// The number of steps the run is expected to have, shown as `[done/total]` on each finished step. The engine does
    /// not know it in advance, the program does.
    #[must_use]
    pub fn total(mut self, steps: usize) -> Self {
        self.total = Some(steps);
        self
    }

    /// Shows what each finished step returned, as indented JSON under its line.
    #[must_use]
    pub fn outputs(mut self, show: bool) -> Self {
        self.outputs = show;
        self
    }

    fn line(&mut self, text: &str) {
        // A reporter that cannot write has nowhere to say so.
        let _ = writeln!(self.out, "{text}");
    }
}

impl<W: Write + Send> Reporter for Lines<W> {
    fn report(&mut self, report: Report<'_>) {
        match report {
            Report::Starting { id } => self.line(&format!("starting {id}")),
            Report::Resuming { id } => self.line(&format!("resuming {id}")),
            Report::Restarting { id } => self.line(&format!("restarting {id}")),
            Report::Finished { id } => {
                self.line(&format!("{id} has finished; its stored result follows"));
            }
            Report::Event(event) => self.event(event),
            Report::Done => {
                let _ = self.out.flush();
            }
        }
    }
}

impl<W: Write + Send> Lines<W> {
    // `Event` can grow: what a reporter does not know it leaves alone.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn event(&mut self, event: &Event) {
        match event {
            Event::TaskStarted {
                task_name,
                replayed: false,
                ..
            } => self.line(&format!("{task_name} started")),
            Event::TaskCompleted {
                task_name,
                replayed: true,
                ..
            } => {
                self.done += 1;
                self.line(&format!("{task_name} replayed{}", self.place()));
            }
            Event::TaskCompleted {
                task_name, result, ..
            } => {
                self.done += 1;
                let described = self
                    .describe
                    .as_ref()
                    .and_then(|describe| describe(task_name, result));
                let text = match described {
                    Some(text) => format!("{task_name} done: {text}{}", self.place()),
                    None => format!("{task_name} done{}", self.place()),
                };
                self.line(&text);
                if self.outputs {
                    let shown = indent_json(result, 4);
                    self.line(&shown);
                }
            }
            Event::TaskFaulted {
                task_name,
                error,
                replayed,
                ..
            } => {
                let again = if *replayed { " (replayed)" } else { "" };
                self.line(&format!("{task_name} failed: {error}{again}"));
            }
            Event::TaskRetried {
                task_name, attempt, ..
            } => self.line(&format!("{task_name} retrying after attempt {attempt}")),
            Event::WorkflowCompleted { instance_id, .. } => {
                self.line(&format!("{instance_id} completed"));
            }
            Event::WorkflowFailed { instance_id, .. } => {
                self.line(&format!("{instance_id} failed"));
            }
            Event::WorkflowCancelled { instance_id, .. } => {
                self.line(&format!("{instance_id} cancelled"));
            }
            _ => {}
        }
    }

    fn place(&self) -> String {
        self.total
            .map_or_else(String::new, |total| format!(" [{}/{total}]", self.done))
    }
}

/// One JSON object per event, one per line: `{"event":"TaskCompleted","instance":"…","task":"…","replayed":false,…}`.
/// For programs that read a run's progress.
pub struct Json<W: Write + Send = std::io::Stderr> {
    out: W,
}

impl Json {
    /// JSON lines on standard error.
    #[must_use]
    pub fn stderr() -> Self {
        Self::new(std::io::stderr())
    }
}

impl<W: Write + Send> Json<W> {
    /// JSON lines written to `out`.
    #[must_use]
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write + Send> Reporter for Json<W> {
    fn report(&mut self, report: Report<'_>) {
        match report {
            Report::Event(event) => {
                let _ = writeln!(self.out, "{}", event_json(event));
            }
            Report::Done => {
                let _ = self.out.flush();
            }
            Report::Starting { .. }
            | Report::Resuming { .. }
            | Report::Restarting { .. }
            | Report::Finished { .. } => {}
        }
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn event_json(event: &Event) -> Value {
    let mut object = json!({
        "event": event.name(),
        "instance": event.instance_id(),
        "replayed": event.replayed(),
    });
    let extra = match event {
        Event::TaskStarted { task_name, .. } => json!({ "task": task_name }),
        Event::TaskCompleted {
            task_name, result, ..
        } => json!({ "task": task_name, "result": result }),
        Event::TaskFaulted {
            task_name, error, ..
        } => json!({ "task": task_name, "error": error }),
        Event::TaskRetried {
            task_name, attempt, ..
        } => json!({ "task": task_name, "attempt": attempt }),
        Event::WorkflowCompleted { output, .. } => json!({ "output": output }),
        Event::WorkflowFailed { error, .. } => json!({ "error": error }),
        _ => Value::Null,
    };
    if let (Some(object), Some(extra)) = (object.as_object_mut(), extra.as_object()) {
        object.extend(extra.clone());
    }
    object
}

/// `value` as pretty JSON, every line indented by `by` spaces.
pub(super) fn indent_json(value: &Value, by: usize) -> String {
    let pad = " ".repeat(by);
    serde_json::to_string_pretty(value)
        .unwrap_or_default()
        .lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
