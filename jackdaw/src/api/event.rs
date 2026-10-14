//! The events of a run, and the stream that delivers them.
use crate::workflow::WorkflowEvent;
use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Something that happened in a workflow instance.
///
/// Steps that are replayed after a restart are reported again with `replayed: true`, so a consumer can tell recorded
/// history from new work. Events from YAML workflows that have no counterpart here arrive as [`Event::Other`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// A new workflow instance began.
    WorkflowStarted {
        /// The instance.
        instance_id: String,
        /// The workflow's identity, `namespace/name version`.
        workflow: String,
        /// The input the workflow was started with.
        input: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A stopped instance was picked up again with [`Engine::resume`](crate::api::Engine::resume).
    WorkflowResumed {
        /// The instance.
        instance_id: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task or child workflow started.
    TaskStarted {
        /// The instance.
        instance_id: String,
        /// The task's name (a child workflow's name for `ctx.run`).
        task_name: String,
        /// `true` if this is recorded history being replayed, not new work.
        replayed: bool,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task or child workflow finished successfully.
    TaskCompleted {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// What it returned, as JSON.
        result: serde_json::Value,
        /// `true` if this is recorded history being replayed, not new work.
        replayed: bool,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task or child workflow failed (after any retries).
    TaskFaulted {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// A description of the failure.
        error: String,
        /// `true` if this is recorded history being replayed, not new work.
        replayed: bool,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// A task failed and its retry policy is trying it again.
    TaskRetried {
        /// The instance.
        instance_id: String,
        /// The task's name.
        task_name: String,
        /// The attempt that just failed, starting at 1.
        attempt: u32,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// The workflow finished successfully. The last event of a successful run.
    WorkflowCompleted {
        /// The instance.
        instance_id: String,
        /// The workflow's output, as JSON.
        output: serde_json::Value,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// The workflow failed. The last event of a failed run.
    WorkflowFailed {
        /// The instance.
        instance_id: String,
        /// A description of the failure.
        error: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// The workflow was cancelled. The last event of a cancelled run.
    WorkflowCancelled {
        /// The instance.
        instance_id: String,
        /// When it happened.
        timestamp: DateTime<Utc>,
    },
    /// An event of the YAML engine that has no dedicated variant.
    Other(WorkflowEvent),
}

impl Event {
    /// The variant's name, e.g. `"TaskCompleted"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorkflowStarted { .. } => "WorkflowStarted",
            Self::WorkflowResumed { .. } => "WorkflowResumed",
            Self::TaskStarted { .. } => "TaskStarted",
            Self::TaskCompleted { .. } => "TaskCompleted",
            Self::TaskFaulted { .. } => "TaskFaulted",
            Self::TaskRetried { .. } => "TaskRetried",
            Self::WorkflowCompleted { .. } => "WorkflowCompleted",
            Self::WorkflowFailed { .. } => "WorkflowFailed",
            Self::WorkflowCancelled { .. } => "WorkflowCancelled",
            Self::Other(event) => other_name(event),
        }
    }

    /// Whether this is recorded history being replayed rather than new work.
    #[must_use]
    pub fn replayed(&self) -> bool {
        match self {
            Self::TaskStarted { replayed, .. }
            | Self::TaskCompleted { replayed, .. }
            | Self::TaskFaulted { replayed, .. } => *replayed,
            Self::WorkflowStarted { .. }
            | Self::WorkflowResumed { .. }
            | Self::TaskRetried { .. }
            | Self::WorkflowCompleted { .. }
            | Self::WorkflowFailed { .. }
            | Self::WorkflowCancelled { .. }
            | Self::Other(_) => false,
        }
    }

    /// The instance the event belongs to.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        match self {
            Self::WorkflowStarted { instance_id, .. }
            | Self::WorkflowResumed { instance_id, .. }
            | Self::TaskStarted { instance_id, .. }
            | Self::TaskCompleted { instance_id, .. }
            | Self::TaskFaulted { instance_id, .. }
            | Self::TaskRetried { instance_id, .. }
            | Self::WorkflowCompleted { instance_id, .. }
            | Self::WorkflowFailed { instance_id, .. }
            | Self::WorkflowCancelled { instance_id, .. } => instance_id,
            Self::Other(event) => event.instance_id(),
        }
    }

    /// Whether this event ends the instance's stream.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::WorkflowCompleted { .. }
                | Self::WorkflowFailed { .. }
                | Self::WorkflowCancelled { .. }
        )
    }

    /// The event as the persistence layer stores it. Replayed events are never stored again.
    pub(crate) fn to_stored(&self) -> Option<WorkflowEvent> {
        Some(match self {
            Self::WorkflowStarted {
                instance_id,
                workflow,
                input,
                timestamp,
            } => WorkflowEvent::WorkflowStarted {
                instance_id: instance_id.clone(),
                workflow_id: workflow.clone(),
                timestamp: *timestamp,
                initial_data: input.clone(),
            },
            Self::WorkflowResumed {
                instance_id,
                timestamp,
            } => WorkflowEvent::WorkflowResumed {
                instance_id: instance_id.clone(),
                timestamp: *timestamp,
            },
            Self::TaskStarted {
                instance_id,
                task_name,
                replayed: false,
                timestamp,
            } => WorkflowEvent::TaskStarted {
                instance_id: instance_id.clone(),
                task_name: task_name.clone(),
                timestamp: *timestamp,
            },
            Self::TaskCompleted {
                instance_id,
                task_name,
                result,
                replayed: false,
                timestamp,
            } => WorkflowEvent::TaskCompleted {
                instance_id: instance_id.clone(),
                task_name: task_name.clone(),
                result: result.clone(),
                timestamp: *timestamp,
                duration_ms: 0,
            },
            Self::TaskFaulted {
                instance_id,
                task_name,
                error,
                replayed: false,
                timestamp,
            } => WorkflowEvent::TaskFaulted {
                instance_id: instance_id.clone(),
                task_name: task_name.clone(),
                error: error.clone(),
                timestamp: *timestamp,
            },
            Self::TaskRetried {
                instance_id,
                task_name,
                attempt,
                timestamp,
            } => WorkflowEvent::TaskRetried {
                instance_id: instance_id.clone(),
                task_name: task_name.clone(),
                attempt: *attempt,
                timestamp: *timestamp,
            },
            Self::WorkflowCompleted {
                instance_id,
                output,
                timestamp,
            } => WorkflowEvent::WorkflowCompleted {
                instance_id: instance_id.clone(),
                final_data: output.clone(),
                timestamp: *timestamp,
                duration_ms: 0,
            },
            Self::WorkflowFailed {
                instance_id,
                error,
                timestamp,
            } => WorkflowEvent::WorkflowFailed {
                instance_id: instance_id.clone(),
                error: error.clone(),
                timestamp: *timestamp,
            },
            Self::WorkflowCancelled {
                instance_id,
                timestamp,
            } => WorkflowEvent::WorkflowCancelled {
                instance_id: instance_id.clone(),
                reason: None,
                timestamp: *timestamp,
            },
            Self::TaskStarted { replayed: true, .. }
            | Self::TaskCompleted { replayed: true, .. }
            | Self::TaskFaulted { replayed: true, .. }
            | Self::Other(_) => return None,
        })
    }

    /// An event of the persistence layer, as the library API reports it.
    #[must_use]
    pub fn from_stored(event: WorkflowEvent) -> Self {
        match event {
            WorkflowEvent::WorkflowStarted {
                instance_id,
                workflow_id,
                timestamp,
                initial_data,
            } => Self::WorkflowStarted {
                instance_id,
                workflow: workflow_id,
                input: initial_data,
                timestamp,
            },
            WorkflowEvent::WorkflowResumed {
                instance_id,
                timestamp,
            } => Self::WorkflowResumed {
                instance_id,
                timestamp,
            },
            WorkflowEvent::TaskStarted {
                instance_id,
                task_name,
                timestamp,
            } => Self::TaskStarted {
                instance_id,
                task_name,
                replayed: false,
                timestamp,
            },
            WorkflowEvent::TaskCompleted {
                instance_id,
                task_name,
                result,
                timestamp,
                ..
            } => Self::TaskCompleted {
                instance_id,
                task_name,
                result,
                replayed: false,
                timestamp,
            },
            WorkflowEvent::TaskFaulted {
                instance_id,
                task_name,
                error,
                timestamp,
            } => Self::TaskFaulted {
                instance_id,
                task_name,
                error,
                replayed: false,
                timestamp,
            },
            WorkflowEvent::TaskRetried {
                instance_id,
                task_name,
                attempt,
                timestamp,
            } => Self::TaskRetried {
                instance_id,
                task_name,
                attempt,
                timestamp,
            },
            WorkflowEvent::WorkflowCompleted {
                instance_id,
                final_data,
                timestamp,
                ..
            } => Self::WorkflowCompleted {
                instance_id,
                output: final_data,
                timestamp,
            },
            WorkflowEvent::WorkflowFailed {
                instance_id,
                error,
                timestamp,
            } => Self::WorkflowFailed {
                instance_id,
                error,
                timestamp,
            },
            WorkflowEvent::WorkflowCancelled {
                instance_id,
                timestamp,
                ..
            } => Self::WorkflowCancelled {
                instance_id,
                timestamp,
            },
            other @ (WorkflowEvent::TaskEntered { .. }
            | WorkflowEvent::TaskCreated { .. }
            | WorkflowEvent::WorkflowCorrelationStarted { .. }
            | WorkflowEvent::WorkflowCorrelationCompleted { .. }
            | WorkflowEvent::WorkflowSuspended { .. }
            | WorkflowEvent::TaskCancelled { .. }
            | WorkflowEvent::TaskSuspended { .. }
            | WorkflowEvent::TaskResumed { .. }) => Self::Other(other),
        }
    }
}

fn other_name(event: &WorkflowEvent) -> &'static str {
    match event {
        WorkflowEvent::WorkflowStarted { .. } => "WorkflowStarted",
        WorkflowEvent::TaskEntered { .. } => "TaskEntered",
        WorkflowEvent::TaskCreated { .. } => "TaskCreated",
        WorkflowEvent::TaskStarted { .. } => "TaskStarted",
        WorkflowEvent::TaskRetried { .. } => "TaskRetried",
        WorkflowEvent::TaskCompleted { .. } => "TaskCompleted",
        WorkflowEvent::WorkflowCompleted { .. } => "WorkflowCompleted",
        WorkflowEvent::WorkflowCorrelationStarted { .. } => "WorkflowCorrelationStarted",
        WorkflowEvent::WorkflowCorrelationCompleted { .. } => "WorkflowCorrelationCompleted",
        WorkflowEvent::WorkflowFailed { .. } => "WorkflowFailed",
        WorkflowEvent::WorkflowCancelled { .. } => "WorkflowCancelled",
        WorkflowEvent::WorkflowSuspended { .. } => "WorkflowSuspended",
        WorkflowEvent::WorkflowResumed { .. } => "WorkflowResumed",
        WorkflowEvent::TaskCancelled { .. } => "TaskCancelled",
        WorkflowEvent::TaskSuspended { .. } => "TaskSuspended",
        WorkflowEvent::TaskResumed { .. } => "TaskResumed",
        WorkflowEvent::TaskFaulted { .. } => "TaskFaulted",
    }
}

/// Everything an instance has emitted so far, shared by all of its [`EventStream`]s.
#[derive(Debug, Default)]
pub(crate) struct EventLog {
    events: Mutex<Vec<Event>>,
    closed: AtomicBool,
    notify: Notify,
}

impl EventLog {
    pub(crate) fn push(&self, event: Event) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
        self.notify.notify_waiters();
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn get(&self, index: usize) -> Option<Event> {
        self.events
            .lock()
            .ok()
            .and_then(|events| events.get(index).cloned())
    }
}

/// The events of one run, from its first event to its terminal one.
///
/// Every call to [`ExecutionHandle::events`](crate::api::ExecutionHandle::events) returns an independent stream that
/// starts from the beginning, so you can watch a run while also awaiting its result. The stream ends after the
/// terminal event ([`Event::WorkflowCompleted`], [`Event::WorkflowFailed`] or [`Event::WorkflowCancelled`]).
///
/// ```
/// use jackdaw::prelude::*;
///
/// // Print each event name until the run is over.
/// async fn watch(mut events: EventStream) {
///     while let Some(event) = events.next().await {
///         println!("{} {}", event.instance_id(), event.name());
///     }
/// }
/// # let _ = watch;
/// ```
///
/// See [observing a run](crate::guide::events) for a complete example.
#[derive(Debug)]
pub struct EventStream {
    log: Arc<EventLog>,
    cursor: usize,
}

impl EventStream {
    pub(crate) fn new(log: Arc<EventLog>) -> Self {
        Self { log, cursor: 0 }
    }

    /// The next event, or `None` once the run has ended and every event has been delivered.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            let notified = self.log.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(event) = self.log.get(self.cursor) {
                self.cursor += 1;
                return Some(event);
            }
            if self.log.closed.load(Ordering::SeqCst) {
                return self.log.get(self.cursor).inspect(|_| self.cursor += 1);
            }
            notified.await;
        }
    }
}
