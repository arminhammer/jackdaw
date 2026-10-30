// `Event` is #[non_exhaustive]: a match outside jackdaw needs a wildcard arm.
use jackdaw::api::Event;

fn describe(value: &Event) -> &'static str {
    match value {
        Event::WorkflowStarted { .. } => "started",
        Event::WorkflowResumed { .. } => "resumed",
        Event::TaskStarted { .. } => "task started",
        Event::TaskCompleted { .. } => "task completed",
        Event::TaskFaulted { .. } => "task faulted",
        Event::TaskRetried { .. } => "task retried",
        Event::WorkflowCompleted { .. } => "completed",
        Event::WorkflowFailed { .. } => "failed",
        Event::WorkflowCancelled { .. } => "cancelled",
        Event::ContinuedAsNew { .. } => "continued",
        Event::Other(_) => "other",
    }
}

fn main() {
    let _ = describe;
}
