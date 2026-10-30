@steps @cancellation
Feature: Cancelling a running workflow
  As a Rust developer
  I want to cancel an instance wherever it is waiting
  So that work that is no longer needed stops

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario Outline: Cancellation interrupts the step the workflow is waiting on
    Given a workflow that is waiting on <step>
    When I call "cancel" on its handle
    Then awaiting the handle returns "Error::Cancelled"
    And the status is "Cancelled"
    And no step after <step> runs

    Examples:
      | step                                  |
      | ctx.sleep(1.hours())                  |
      | ctx.wait_for_event("paid")            |
      | ctx.wait_condition on the state       |

  @todo
  Scenario: The workflow can observe cancellation and clean up
    Given a workflow that matches "Err(StepError::Cancelled)" on its current step and calls "ReleaseStock" in a non-cancellable scope
    When it is cancelled
    Then "release_stock" runs to completion
    And the instance ends "Cancelled"

  @todo
  Scenario: A running task sees its cancellation
    Given a task that loops until "TaskContext::cancelled()" resolves
    When its instance is cancelled
    Then the task stops within 1 second

  @todo @crash
  Scenario: A cancelled instance stays cancelled after a restart
    Given a cancelled instance
    When a new engine recovers the store
    Then the instance is not resumed
