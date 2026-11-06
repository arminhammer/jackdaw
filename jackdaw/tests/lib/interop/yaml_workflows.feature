@interop @yaml
Feature: Rust and YAML workflows on one engine
  As a library user
  I want to run Open Workflow YAML from Rust and call Rust from YAML
  So that both styles share storage, events and tasks

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: Rust runs a YAML workflow
    Given the YAML workflow "hello-world.sw.yaml" loaded with "Workflow::from_yaml"
    When I execute it from Rust
    Then the output is "Hello, World!"

  @implemented
  Scenario: A Rust workflow runs a YAML workflow as a step
    Given a Rust workflow that calls "ctx.run_definition(&yaml_workflow, &input)"
    When I execute the Rust workflow
    Then the YAML workflow's output is returned to the Rust workflow as a typed value

  @implemented
  Scenario: YAML calls a Rust task by name
    Given the Rust task "payments.charge" registered on the engine
    And a YAML workflow with a "call: payments.charge" task
    When I execute the YAML workflow
    Then the Rust task runs with the YAML task's arguments as its typed input
    And its output becomes the YAML task's output

  Scenario: A YAML run reports its tasks as events while it runs
    Given a YAML workflow with a "call: payments.charge" task
    When I execute the YAML workflow and watch its events
    Then the events are "WorkflowStarted", "TaskStarted", "TaskCompleted", "WorkflowCompleted"
    And the "TaskCompleted" event names the task "charge"

  Scenario: A YAML run that fails reports the failed task
    Given a YAML workflow that raises an error in its task "explode"
    When I execute the YAML workflow and watch its events
    Then the events are "WorkflowStarted", "TaskStarted", "TaskFaulted", "WorkflowFailed"
    And the "TaskFaulted" event names the task "explode"

  Scenario: Starting a finished YAML run again under its id returns its output
    Given a YAML workflow with a "call: payments.charge" task
    When I execute the YAML workflow twice under the id "daily"
    Then both runs return the same output
    And the payment was made once

  Scenario: A finished YAML instance can be purged and its id used again
    Given a YAML workflow with a "call: payments.charge" task
    When I execute the YAML workflow under the id "daily", purge it, and execute it again
    Then the payment was made twice

  @implemented
  Scenario: YAML and Rust instances share the history store
    When I execute one YAML and one Rust workflow
    Then "Engine::history" returns events for both instances

  @implemented
  Scenario: YAML instances are listed and described like Rust ones
    When I execute one YAML and one Rust workflow
    Then "Engine::list" returns both instances with their workflows
    And "Engine::status" and "Engine::describe" report the YAML instance as completed

  @implemented
  Scenario: YAML workflows are durable on the new engine
    Given a YAML workflow with a "wait" of 1 hour between two "set" tasks
    When I execute it and drop the engine during the wait
    And a new engine calls "Engine::recover()"
    Then the YAML instance completes
    And the first "set" task is not re-executed

  @implemented
  Scenario: Invalid YAML is rejected before it runs
    Given a YAML workflow that refers to an undefined task in a "then" directive
    When I execute it from Rust
    Then it fails with "Error::Invalid" naming the directive
    And no task of the workflow ran

  @implemented
  Scenario: The YAML interpreter is optional
    Given "jackdaw" built without the "yaml" feature
    Then "Workflow::from_yaml" does not exist
    And "#[catalog]" fails naming the "yaml" feature
    And the native Rust API works
