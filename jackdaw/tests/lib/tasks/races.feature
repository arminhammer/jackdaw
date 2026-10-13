@steps @races
Feature: Racing steps
  As a Rust developer
  I want to take whichever step finishes first
  So that I can implement deadlines, fallbacks and "first response wins"

  Background:
    Given an engine with a Redb store and a controllable clock

  @implemented
  Scenario: select! takes the first branch to complete
    Given a workflow whose run does:
      """rust
      Ok(jackdaw::select! {
          approval = ctx.wait_for_event::<Approved>("approved") => format!("approved by {}", approval?.by),
          _ = ctx.sleep(24.hours()) => "expired".to_string(),
      })
      """
    When the event "approved" from "ada" is sent after 1 hour
    Then the workflow completes with "approved by ada"

  @implemented
  Scenario: The timer wins when nothing arrives
    Given the same workflow
    When 24 hours pass without the event
    Then the workflow completes with "expired"

  @implemented
  Scenario: The winner is recorded and replay never starts the loser
    Given a completed instance where the event won the race
    When I replay the instance
    Then the replay takes the event branch
    And no timer is started during the replay

  @implemented
  Scenario: race returns which side won as Either
    Given a workflow that does "match race(ctx.call(Primary, &q), ctx.call(Fallback, &q)).await { Either::Left(p) => .., Either::Right(f) => .. }"
    And "Primary" takes 2 seconds and "Fallback" takes 100 ms
    When I execute it
    Then the workflow uses the "Fallback" result

  @todo
  Scenario: The losing task is cancelled
    Given a race between a 100 ms task and a task that waits for cancellation
    When the 100 ms task wins
    Then the losing task observed cancellation and did not complete

  @implemented
  Scenario: tokio::select! in a run body is a compile error
    Given a "#[run]" body that uses "tokio::select!"
    When I compile it
    Then the error suggests "jackdaw::select!"

  @todo @crash
  Scenario: A race in progress resumes with the same deadline
    Given the select! of an event and a 24-hour timer has waited 20 hours
    When the engine is dropped and a new engine resumes the instance
    And 4 more hours pass without the event
    Then the workflow completes with "expired"

  @todo @crash
  Scenario: An event that arrived during the crash still wins
    Given the select! of an event and a 24-hour timer
    When the event is sent and the engine is dropped before the race resolves
    And a new engine resumes the instance
    Then the event branch wins
