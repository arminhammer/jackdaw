@interop @testing
Feature: Testing workflows
  As a library user
  I want a test engine with a virtual clock and replay checking
  So that my workflow tests are fast and catch determinism bugs

  @implemented
  Scenario: TestEngine runs on a virtual clock
    Given a "TestEngine" with a workflow that sleeps 7 days between two tasks
    When I execute it
    Then it completes in under one second of wall-clock time
    And "clock().elapsed()" is 7 days

  @implemented
  Scenario: TestEngine replays each run to verify it
    Given a "TestEngine" with self-replay enabled
    And a workflow that reads "SystemTime::now" under "#[allow_nondeterminism]"
    When I execute it
    Then the run fails with "Error::NonDeterminism"

  @implemented
  Scenario: Self-replay can be turned off
    Given a "TestEngine" built with "self_replay(false)"
    When I execute the same nondeterministic workflow
    Then it completes

  @implemented
  Scenario: Tasks can be unit-tested as plain functions
    Given the task function "charge"
    When I call "charge(order).await" directly in a test
    Then it returns its result without an engine

  @todo
  Scenario: A task can be replaced with a fake in tests
    Given a "TestEngine" built with "replace_task(Charge, |order| async { Ok(\"fake\".into()) })"
    When a workflow that calls "Charge" runs
    Then the step returns "fake"
    And the real "charge" function is not called

  @todo
  Scenario: Tests can simulate a crash
    Given a "TestEngine" with a workflow of three steps
    When I call "TestEngine::crash_after_step(2)" and execute it
    And I call "TestEngine::restart()"
    Then the workflow completes
    And steps 1 and 2 are not re-executed
