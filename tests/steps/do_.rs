use crate::CtKWorld;
use crate::common::{WorkflowStatus, parse_docstring};
use cucumber::then;
use serde_json::Value;

#[then(expr = "the workflow should complete with output:")]
async fn then_output(world: &mut CtKWorld, step: &cucumber::gherkin::Step) {
    assert_eq!(
        world.workflow_status,
        Some(WorkflowStatus::Completed),
        "Expected workflow to complete"
    );
    let expected: Value =
        serde_yaml::from_str(&parse_docstring(step.docstring.as_ref().unwrap())).unwrap();
    assert_eq!(world.workflow_output.as_ref().unwrap(), &expected);
}
