@operations @lifecycle
Feature: Graceful shutdown
  As an operator
  I want to stop a process without corrupting or losing work
  So that deploys and scale-downs are safe

  @todo
  Scenario: Shutdown waits for in-flight steps
    Given an engine with a Redb store running a task that takes 2 seconds
    When I call "Engine::shutdown()" with a grace period of 10 seconds
    Then the task finishes and its result is persisted
    And "shutdown" returns after the task completes
    And the instance is left "Running" for the next process to recover

  @todo
  Scenario: Shutdown stops accepting work
    Given an engine that is shutting down
    When I call "execute"
    Then it fails with an error that says the engine is shutting down

  @todo
  Scenario: Shutdown does not wait for long sleeps
    Given an instance sleeping for 1 hour
    When I call "Engine::shutdown()"
    Then it returns promptly
    And the instance resumes with the remaining sleep in the next process

  @todo
  Scenario: Steps still running after the grace period are abandoned safely
    Given a task that takes 1 minute
    When I call "Engine::shutdown()" with a grace period of 1 second
    Then "shutdown" returns after about 1 second
    And the step is not recorded as completed
    And the next process runs the task again with the same idempotency key

  @todo
  Scenario: Dropping the engine flushes the store
    Given an engine with a Redb store
    When a step completes and the engine is dropped immediately
    Then a new engine on the same path sees the step as completed
