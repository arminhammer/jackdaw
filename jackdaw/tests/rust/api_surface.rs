//! Everything a user needs is reachable from `jackdaw::prelude::*`. This module only has to compile: it names each
//! public type once, so a missing re-export or a rename is a build error here first.
#![allow(unused)]

use jackdaw::prelude::*;

struct Surface {
    // Engine and its builders
    engine: Option<Engine>,
    builder: Option<EngineBuilder>,
    test_engine: Option<jackdaw::testing::TestEngine>,
    // Running things
    handle: Option<ExecutionHandle<serde_json::Value>>,
    events: Option<EventStream>,
    event: Option<Event>,
    status: Option<Status>,
    divergence: Option<Divergence>,
    // Defining things
    workflow: Option<Workflow<serde_json::Value, serde_json::Value>>,
    // Inside tasks and workflows
    task_context: Option<TaskContext>,
    workflow_context: Option<WorkflowContextView>,
    task_options: Option<TaskOptions>,
    retry: Option<Retry>,
    // Errors
    error: Option<Error>,
    fault: Option<Fault>,
    task_error: Option<TaskError>,
    step_error: Option<StepError>,
    workflow_error: Option<WorkflowError>,
    // Providers
    redb: Option<Redb>,
}

// Traits and macros a user writes against.
fn _traits<T: TaskDef>() {}
fn _workflows<W: WorkflowDef>() {}
fn _catalogs<C: CatalogDef>() {}

fn _results() -> (WorkflowResult<()>, Result<(), TaskError>) {
    (Ok(()), Ok(()))
}

fn _durations() {
    // Duration helpers on integers: `10.millis()`, `30.secs()`, `24.hours()`.
    let _ = (10.millis(), 30.secs(), 24.hours());
}

fn _constants() {
    let _ = (
        error_types::COMMUNICATION,
        error_types::RUNTIME,
        error_types::TIMEOUT,
        error_types::VALIDATION,
    );
}
