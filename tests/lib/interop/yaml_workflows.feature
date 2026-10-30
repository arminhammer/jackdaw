@interop @yaml
Feature: Rust and YAML workflows on one engine
  As a library user
  I want to run Serverless Workflow YAML from Rust and call Rust from YAML
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

  @implemented
  Scenario: YAML and Rust instances share the history store
    When I execute one YAML and one Rust workflow
    Then "Engine::history" returns events for both instances

  @todo
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

  @todo
  Scenario: The YAML interpreter is optional
    Given "jackdaw" built without the "yaml" feature
    Then "Workflow::from_yaml" does not exist
    And the native Rust API works
