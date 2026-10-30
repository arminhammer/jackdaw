// A signal handler changes state synchronously; it cannot run steps.
use jackdaw::prelude::*;

#[task]
async fn charge(cents: u64) -> Result<String, TaskError> {
    Ok(format!("ch-{cents}"))
}

#[workflow(namespace = "ui", name = "signal-calls-task", version = "1.0.0")]
pub struct W {
    approved: bool,
}

#[workflow_methods]
impl W {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { approved: false }
    }

    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.wait_condition(|w| w.approved).await?;
        Ok(())
    }

    #[signal]
    fn approve(&mut self, ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        let _ = ctx.call(Charge, &5);
        self.approved = true;
    }
}

fn main() {}
