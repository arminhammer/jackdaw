use crate::NestedWorkflowWorld;
use crate::common::parse_docstring;
use cucumber::{given, then, when};
use serverless_workflow_core::models::workflow::WorkflowDefinition;

#[given(regex = r"^the following workflows are registered:$")]
async fn given_workflows_registered(
    world: &mut NestedWorkflowWorld,
    step: &cucumber::gherkin::Step,
) {
    let table = step.table.as_ref().expect("Table required");

    // Get the base path for workflow files
    let base_path = "tests/fixtures/nested-workflows";

    // Skip header row, process data rows
    for row in table.rows.iter().skip(1) {
        let namespace = &row[0];
        let name = &row[1];
        let version = &row[2];
        let file = &row[3];

        // Read the workflow file
        let file_path = format!("{}/{}", base_path, file);
        let workflow_yaml = std::fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("Failed to read workflow file {}: {}", file_path, e));

        // Store in registry with key "namespace/name/version"
        let key = format!("{}/{}/{}", namespace, name, version);
        world.workflow_registry.insert(key, workflow_yaml);
    }

    println!("Registered {} workflows", world.workflow_registry.len());
}

#[when(regex = r#"^I execute workflow "([^"]+)" with input:$"#)]
async fn when_execute_workflow(
    world: &mut NestedWorkflowWorld,
    workflow_ref: String,
    step: &cucumber::gherkin::Step,
) {
    let input_text = parse_docstring(step.docstring.as_ref().unwrap());
    let input: serde_json::Value = serde_json::from_str(&input_text)
        .unwrap_or_else(|e| panic!("Failed to parse input JSON: {}", e));

    world.workflow_input = Some(input.clone());

    // Look up the workflow in the registry
    let workflow_yaml = world
        .workflow_registry
        .get(&workflow_ref)
        .unwrap_or_else(|| panic!("Workflow {} not found in registry", workflow_ref));

    // Parse the workflow
    let workflow: WorkflowDefinition = serde_yaml::from_str(workflow_yaml)
        .unwrap_or_else(|e| panic!("Failed to parse workflow YAML: {}", e));

    // Get the engine
    let engine = world.engine.as_ref().expect("Engine not initialized");

    // Before starting the workflow, we need to register all workflows with the engine
    // so that nested workflows can be found during execution
    for (key, yaml) in &world.workflow_registry {
        let wf: WorkflowDefinition = serde_yaml::from_str(yaml)
            .unwrap_or_else(|e| panic!("Failed to parse workflow {}: {}", key, e));

        // Register the workflow with the engine
        engine
            .register_workflow(wf.clone())
            .await
            .unwrap_or_else(|e| panic!("Failed to register workflow {}: {}", key, e));
    }

    // Execute the workflow
    match engine.start_with_input(workflow, input).await {
        Ok((instance_id, _output)) => {
            world.instance_id = Some(instance_id.clone());

            // Wait for the workflow to complete
            match engine
                .wait_for_completion(&instance_id, std::time::Duration::from_secs(30))
                .await
            {
                Ok(output) => {
                    world.workflow_output = Some(output);
                    world.workflow_status = Some(crate::common::WorkflowStatus::Completed);
                }
                Err(e) => {
                    world.error_message = Some(format!("Workflow execution failed: {}", e));
                    world.workflow_status =
                        Some(crate::common::WorkflowStatus::Faulted(e.to_string()));
                }
            }
        }
        Err(e) => {
            world.error_message = Some(format!("Failed to start workflow: {}", e));
            world.workflow_status = Some(crate::common::WorkflowStatus::Faulted(e.to_string()));
        }
    }
}

#[then("the workflow should complete successfully")]
async fn then_workflow_completes(world: &mut NestedWorkflowWorld) {
    if let Some(ref error) = world.error_message {
        panic!("Workflow failed: {}", error);
    }

    assert_eq!(
        world.workflow_status,
        Some(crate::common::WorkflowStatus::Completed),
        "Workflow did not complete successfully"
    );
}

#[then("the workflow output should be:")]
async fn then_workflow_output(world: &mut NestedWorkflowWorld, step: &cucumber::gherkin::Step) {
    let expected_text = parse_docstring(step.docstring.as_ref().unwrap());
    let expected: serde_json::Value = serde_json::from_str(&expected_text)
        .unwrap_or_else(|e| panic!("Failed to parse expected output JSON: {}", e));

    let actual = world.workflow_output.as_ref().expect("No workflow output");

    assert_eq!(
        actual,
        &expected,
        "Workflow output mismatch.\nExpected: {}\nActual: {}",
        serde_json::to_string_pretty(&expected).unwrap(),
        serde_json::to_string_pretty(actual).unwrap()
    );
}
