@steps @timers
Feature: Sleeping and timeouts in Rust workflows
  As a Rust developer
  I want durable sleeps and timeouts on any step
  So that workflows can wait for days without holding resources

  Background:
    Given an engine with a Redb store and a controllable clock

  @implemented
  Scenario: ctx.sleep pauses the workflow
    Given a workflow that calls "ctx.sleep(3.days())" between two task calls
    When I execute it
    Then the second task has not run after 71 hours
    And it has run after 72 hours

  @implemented
  Scenario: Sleeps can be computed from recorded time
    Given a workflow that sleeps for the difference between an input deadline and "ctx.now()"
    When I execute it with a deadline 2 hours away
    Then the workflow wakes after 2 hours
    And a replay computes the same duration

  @implemented
  Scenario: A timeout applies to any step
    Given a workflow that does "ctx.wait_for_event::<Paid>(\"paid\").timeout(1.hours()).await"
    When no event arrives for 1 hour
    Then the step returns "Err(StepError::Timeout { .. })"

  @implemented
  Scenario: A timeout applies to a child workflow
    Given a workflow that does "ctx.run(Slow::run, &input).timeout(5.mins()).await"
    And "Slow" takes 10 minutes
    When I execute it
    Then the step times out after 5 minutes

  @implemented
  Scenario: A sleep that is in a test runs instantly
    Given a "TestEngine" running a workflow that sleeps 30 days
    When I execute it
    Then it completes in under a second of wall-clock time

  @todo @crash
  Scenario: A sleep resumes with only the remaining time
    Given a workflow sleeping 1 hour that has slept 40 minutes
    When the engine is dropped and a new engine resumes the instance
    Then the workflow wakes 20 minutes later

  @todo @crash
  Scenario: A step timeout keeps its deadline across a crash
    Given a step with ".timeout(1.hours())" that has been waiting 50 minutes
    When the engine is dropped and a new engine resumes the instance
    Then the step times out 10 minutes later

  @todo
  Scenario: A whole workflow can have a deadline
    Given a workflow executed with an execution timeout of 1 day
    When it is still running after 1 day
    Then it ends with "Error::Timeout"
    And its status is "Faulted"
