@api @workflows
Feature: Workflows written in Rust
  As a library user
  I want to write workflows as ordinary async Rust
  So that control flow is "if", "match", loops and "?"

  Background:
    Given an in-memory engine

  @implemented
  Scenario: The quickstart workflow runs to completion
    Given the "OrderWorkflow" from the crate documentation with tasks "Charge" and "Ship"
    When I execute "OrderWorkflow::run" with order "o-1"
    And I await the handle
    Then the result is a shipment with tracking "trk-ch-o-1"

  @implemented
  Scenario: Execute returns before the workflow finishes
    Given a workflow that waits for a signal before finishing
    When I execute it
    Then I get a handle immediately
    And "ExecutionHandle::id" is a non-empty instance id
    And "Engine::status" of that id is "Running"

  @implemented
  Scenario: A workflow's output type is checked at compile time
    Given a workflow whose "#[run]" returns "WorkflowResult<u64>"
    When the caller awaits the handle as "String"
    Then it fails to compile with a type mismatch

  @implemented
  Scenario: A workflow needs exactly one run method
    Given a "#[workflow_methods]" impl without "#[run]"
    When I compile it
    Then it fails with "a workflow needs a `#[run]` method"

  @implemented
  Scenario: A workflow runs a child workflow
    Given a parent workflow that runs the child "Refund" with input 5
    When I execute the parent
    Then the child runs as its own instance
    And the parent's result contains the child's output

  @implemented
  Scenario: A child workflow's failure surfaces in the parent
    Given a child workflow that fails with a validation fault
    When the parent runs it
    Then the parent's step fails with that fault

  @implemented
  Scenario: Steps over a list run in parallel with a limit
    Given a task that tracks how many copies of it run at the same time
    When a workflow calls "ctx.for_each(&items, Task).concurrency(3)" over 10 items
    Then all 10 results come back in input order
    And no more than 3 copies ran at the same time

  @implemented
  Scenario: Racing two steps takes the first to finish
    Given a workflow that races a 1-second sleep against "wait_for_event(\"approved\")"
    When the event "approved" arrives after 100 ms
    Then the race resolves to the event side
    And replaying the instance takes the same side

  @implemented
  Scenario Outline: Nondeterministic values are recorded
    Given a workflow that reads "<call>" and returns it
    When I execute it and record the result
    And I replay the instance from its journal
    Then the replay returns the same value

    Examples:
      | call         |
      | ctx.now()    |
      | ctx.random() |
      | ctx.uuid()   |

  @implemented
  Scenario: A workflow can read its own state from a query while it runs
    Given a workflow with a "#[query]" that returns its step counter
    When the workflow has finished 2 of 3 steps
    Then querying it returns 2

  @todo
  Scenario: A workflow continues as new to bound its history
    Given a workflow that processes one batch and then calls "ctx.continue_as_new(next_cursor)"
    When it has processed 1000 batches
    Then the current run's journal holds the steps of one batch only
    And all runs share the same workflow instance id
    And "Engine::history" can follow the chain of runs
    And the handle the caller holds resolves when the last run completes
