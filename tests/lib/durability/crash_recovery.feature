@durability @crash
Feature: Workflows resume after a crash
  As a library user
  I want a workflow interrupted by a crash to continue where it stopped
  So that every completed task runs exactly once

  # The runner executes every scenario in this file once per durable store:
  # Redb, Sqlite and Postgres. "the store" below means the store of the current run.

  Background:
    Given a durable store shared by two engine lifetimes

  @implemented
  Scenario: Resume continues from the journal
    Given a workflow with tasks "A", "B" and "C" that count their invocations
    And "B" blocks until it is released
    When I execute the workflow
    And "A" has completed
    And the engine is dropped while "B" is running
    And a new engine resumes the instance with "Engine::resume::<W>(id)"
    And "B" is released
    Then the workflow completes
    And "A" ran 1 time
    And "C" ran 1 time
    And the "WorkflowResumed" event was emitted

  @implemented
  Scenario: Resume rejects the wrong workflow type
    Given an instance of workflow "shop/order@1.0.0" in the store
    When a new engine calls "Engine::resume::<RefundWorkflow>(id)"
    Then it fails with "Error::Invalid" saying the instance belongs to "shop/order@1.0.0"

  @todo
  Scenario: Running instances are recovered automatically at startup
    Given three instances in the store with statuses "Running", "Running" and "Completed"
    When a new engine is built on the store and "Engine::recover()" is called
    Then the two running instances are resumed
    And the completed instance is not touched
    And "recover" returns the ids it resumed

  @todo
  Scenario: Recovery reports instances it cannot resume
    Given a running instance of workflow "shop/order@0.9.0" in the store
    And the new engine only registers "shop/order@1.0.0"
    When "Engine::recover()" is called
    Then the instance is reported as unrecoverable with its workflow id and version
    And it stays "Running" in the store
    And the other instances are still recovered

  @todo
  Scenario Outline: Every kind of step resumes without repeating completed work
    Given a workflow whose second step is <step>
    And the engine is dropped while the second step is in flight
    When a new engine recovers the instance
    Then the workflow completes
    And the first step's task ran 1 time
    And the second step ran to completion exactly once

    Examples:
      | step                                                  |
      | a task call                                           |
      | a task call that is retrying after a failure          |
      | ctx.sleep(1.hours())                                  |
      | ctx.wait_for_event("paid")                            |
      | ctx.wait_condition on a signal-driven flag            |
      | a child workflow                                      |
      | ctx.for_each over 5 items with concurrency 2          |
      | race of a sleep and an event                          |
      | ctx.run_definition of a YAML workflow                 |

  @todo
  Scenario: Child workflows are recovered with their parent
    Given a parent that is waiting on a running child
    And the engine is dropped
    When a new engine recovers
    Then the child resumes
    And the parent resumes and receives the child's result
    And the child is not started a second time

  @implemented
  Scenario: A task completed just before a crash is not re-run
    Given a task whose completion is persisted
    And the engine is dropped right after the persist
    When a new engine resumes the instance
    Then the task does not run again

  @todo
  Scenario: A task that was running at the crash is retried, with the same idempotency key
    Given a task that records its idempotency key and was running when the engine was dropped
    When a new engine resumes the instance
    Then the task runs again
    And it receives the same idempotency key as the interrupted attempt
