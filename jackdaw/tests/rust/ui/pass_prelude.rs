// Everything an application needs comes from one import.
#![allow(dead_code, unused_imports)]
use jackdaw::prelude::*;

type Names = (
    Engine,
    EngineBuilder,
    ExecutionHandle<()>,
    WorkflowContext<()>,
    TaskContext,
    TaskError,
    WorkflowError,
    WorkflowResult<()>,
    Error,
    Retry,
    Event,
    Status,
    Redb,
);

fn durations() -> std::time::Duration {
    5.secs()
}

fn takes_duration_ext<T: DurationExt>(value: T) -> std::time::Duration {
    value.minutes()
}

#[task]
async fn noop(_: ()) -> Result<(), TaskError> {
    Ok(())
}

#[tasks]
impl Holder {
    #[task]
    async fn held(self: std::sync::Arc<Self>, _: ()) -> Result<(), TaskError> {
        Ok(())
    }
}

struct Holder;

#[workflow(namespace = "ui", name = "prelude", version = "1.0.0")]
struct W;

#[workflow_methods]
impl W {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        Ok(ctx.call(Noop, &()).await?)
    }
}

fn main() {
    let _ = catalog_attribute_exists;
}

// `#[catalog]` is in the prelude too; naming it is enough here.
#[allow(unused)]
fn catalog_attribute_exists() {
    let _ = stringify!(catalog);
}
