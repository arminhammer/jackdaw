@operations @schedules
Feature: Scheduled workflows
  As a library user
  I want to run workflows on a schedule
  So that recurring jobs are durable too

  Background:
    Given an engine with a Redb store and a controllable clock

  @todo
  Scenario: A cron schedule starts a workflow at each tick
    Given a schedule "nightly-report" for "ReportWorkflow::run" with cron "0 2 * * *"
    When the clock moves from "2026-01-01T00:00:00Z" to "2026-01-03T03:00:00Z"
    Then "ReportWorkflow" was started twice
    And the instances were started at "2026-01-01T02:00:00Z" and "2026-01-02T02:00:00Z"

  @todo
  Scenario: An interval schedule starts a workflow at fixed intervals
    Given a schedule for "PollWorkflow::run" every 15 minutes
    When the clock advances 1 hour
    Then "PollWorkflow" was started 4 times

  @todo
  Scenario Outline: Missed ticks follow the catch-up policy
    Given a schedule with cron "0 * * * *" and catch-up policy "<policy>"
    And the engine was down from "01:30" to "04:30"
    When a new engine starts at "04:30"
    Then <count> instances are started for the missed ticks

    Examples:
      | policy   | count |
      | skip     | 0     |
      | one      | 1     |
      | all      | 3     |

  @todo
  Scenario: The last fire time is durable
    Given a schedule that fired at "02:00"
    When the engine restarts at "02:10"
    Then the "02:00" tick is not fired again

  @todo
  Scenario: Overlapping runs follow the overlap policy
    Given a schedule every minute for a workflow that takes 3 minutes
    And the overlap policy is "skip"
    When the clock advances 5 minutes
    Then no two instances of the schedule run at the same time

  @todo
  Scenario: Scheduled instance ids are deterministic
    Given a schedule "nightly-report"
    When it fires for "2026-01-01T02:00:00Z" on two engines that share a store
    Then one instance "nightly-report/2026-01-01T02:00:00Z" exists

  @todo @yaml
  Scenario: YAML workflows with a schedule are scheduled by the engine
    Given a YAML workflow with "schedule: { cron: \"0 2 * * *\" }"
    When it is registered on the engine
    Then it is started at each tick like a native schedule

  @todo
  Scenario: Schedules can be paused, resumed and removed
    Given a running schedule
    When I pause it
    Then no ticks fire until I resume it
    When I remove it
    Then no further ticks fire after a restart
