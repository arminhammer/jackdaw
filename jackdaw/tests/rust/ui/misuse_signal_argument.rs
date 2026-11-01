// A signal's argument type comes from its handler.
use jackdaw::prelude::*;

#[workflow(namespace = "ui", name = "signal-argument", version = "1.0.0")]
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
    pub fn approve(&mut self, _ctx: &mut SyncWorkflowContext<Self>, by: String) {
        let _ = by;
        self.approved = true;
    }
}

async fn send(handle: ExecutionHandle<()>) -> Result<(), Error> {
    handle.signal(W::approve, 42).await
}

fn main() {
    let _ = send;
}
