@operations @workers
Feature: Several engine processes share one store safely
  As an operator
  I want to run more than one process against a shared database
  So that I can scale out and survive the loss of a machine

  Background:
    Given a Postgres store shared by engine processes "worker-1" and "worker-2"
    And both processes register the same workflows

  @todo
  Scenario: An instance is owned by one engine at a time
    Given "worker-1" is running instance "o-1"
    When "worker-2" calls "Engine::recover()"
    Then "worker-2" does not resume "o-1"
    And every task of "o-1" runs once

  @todo
  Scenario: Ownership moves when the owner dies
    Given "worker-1" owns "o-1" with a lease of 30 seconds
    When "worker-1" stops heartbeating
    And 30 seconds pass
    And "worker-2" calls "Engine::recover()"
    Then "worker-2" takes over "o-1"
    And "o-1" completes
    And completed tasks of "o-1" are not run again

  @todo
  Scenario: An engine that lost its lease stops writing
    Given "worker-1" owned "o-1" and was paused longer than its lease
    And "worker-2" has taken over "o-1"
    When "worker-1" wakes up and tries to record a step for "o-1"
    Then the write is rejected because the lease is held by "worker-2"
    And "worker-1" stops running "o-1"

  @todo
  Scenario: A signal reaches the instance whichever engine receives it
    Given "worker-1" owns "o-1"
    When a signal for "o-1" is sent through "worker-2"
    Then "o-1" receives the signal

  @todo
  Scenario: Without leases, one store must have one engine
    Given a store backend that does not support leases
    When a second engine is built on it while the first is running
    Then "build" fails with an error that says the store is already in use
