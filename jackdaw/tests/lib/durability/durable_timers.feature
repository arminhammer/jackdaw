@durability @timers
Feature: Durable timers survive restarts
  As a library user
  I want "ctx.sleep" to mean "wake me at this time"
  So that a restart does not reset a 30-day timer

  Background:
    Given an engine with a Redb store and a controllable clock

  @implemented
  Scenario: A sleep completes after its duration
    Given a workflow that calls "ctx.sleep(10.secs())" and then returns "done"
    When I execute it
    And the clock advances 10 seconds
    Then the workflow completes with "done"

  @todo @crash
  Scenario: A resumed sleep waits only for the time that is left
    Given a workflow that calls "ctx.sleep(1.hours())" and then returns "done"
    When I execute it
    And the clock advances 40 minutes
    And the engine is dropped
    And a new engine on the same store resumes the instance
    Then the workflow is still sleeping
    When the clock advances 20 minutes
    Then the workflow completes with "done"

  @todo @crash
  Scenario: A timer that fired while the process was down fires on resume
    Given a workflow that calls "ctx.sleep(5.mins())" and then returns "done"
    When I execute it
    And the engine is dropped
    And the clock advances 1 hour
    And a new engine on the same store resumes the instance
    Then the workflow completes with "done" without waiting

  @todo
  Scenario: The fire time is recorded in the journal when the sleep starts
    Given a workflow that calls "ctx.sleep(1.hours())"
    When I execute it at "2026-01-01T00:00:00Z"
    Then the journal records the sleep step with fire time "2026-01-01T01:00:00Z"

  @todo
  Scenario: A replayed sleep does not wait at all
    Given a completed instance that slept for 1 hour
    When I call "Engine::replay_all"
    Then the replay finishes without advancing the clock

  @implemented
  Scenario: The test engine's virtual clock skips sleeps
    Given a "TestEngine" with a workflow that sleeps 30 days
    When I execute it
    Then it completes immediately in wall-clock time
    And "TestEngine::clock().elapsed()" is 30 days
