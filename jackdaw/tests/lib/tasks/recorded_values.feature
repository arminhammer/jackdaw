@steps @determinism
Feature: Time, randomness and ids in workflows
  As a Rust developer
  I want the clock and randomness through ctx
  So that replay sees the same values the first run saw

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario Outline: Recorded values are stable across replay
    Given a workflow that reads "<call>" and passes it to a task
    When I execute it and replay it
    Then the replay passes the same value to the task
    And "Engine::replay_all" reports no divergence

    Examples:
      | call         |
      | ctx.now()    |
      | ctx.random() |
      | ctx.uuid()   |

  @implemented
  Scenario: Recorded values drive control flow deterministically
    Given a workflow that does "if ctx.random() < 0.1 { ctx.call(Audit, &order).await?; }"
    When I execute 100 instances and replay each one
    Then every replay makes the same audit decision as its first run

  @implemented
  Scenario: ctx.now is the recorded time, not the wall clock
    Given a workflow that reads "ctx.now()" before and after a 1-hour sleep on a virtual clock
    When I execute it
    Then the two values are 1 hour apart

  @implemented
  Scenario: Wall-clock and random APIs are rejected at compile time
    Given a "#[run]" body that calls "std::time::SystemTime::now()"
    When I compile it
    Then the error suggests "ctx.now()"
