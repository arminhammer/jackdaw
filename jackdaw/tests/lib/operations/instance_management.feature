@operations @instances
Feature: Finding and managing workflow instances
  As an operator of a service built on jackdaw
  I want to look up, list, control and clean up instances
  So that I can run the engine behind an API and in production

  Background:
    Given an engine with a Redb store

  @todo
  Scenario: An instance can be started with a caller-chosen id
    When I call "Engine::execute_with_id(\"order-42\", OrderWorkflow::run, order)"
    Then the handle's id is "order-42"
    And "Engine::status(\"order-42\")" is "Running" or "Completed"

  @todo
  Scenario: Starting the same id twice returns the same instance
    Given I started "order-42" with "execute_with_id"
    When I call "execute_with_id(\"order-42\", ..)" again with the same input
    Then no second instance is created
    And the returned handle refers to the existing "order-42"
    And the workflow's tasks ran once

  @todo
  Scenario: Starting the same id with a different workflow is an error
    Given I started "order-42" as "OrderWorkflow"
    When I call "execute_with_id(\"order-42\", RefundWorkflow::run, ..)"
    Then it fails with "Error::Invalid" naming the existing workflow

  @todo
  Scenario: Concurrent starts with the same id create one instance
    When 10 tasks call "execute_with_id(\"order-42\", ..)" at the same time
    Then exactly one instance "order-42" exists
    And all 10 handles resolve to the same result

  @todo
  Scenario: A handle can be obtained by id
    Given a running "Approval" instance started by another part of the program
    When I call "Engine::handle::<Approval>(id)"
    Then I can signal, query, cancel and await it through that handle

  @todo
  Scenario: A handle by id from a fresh engine attaches to a stored instance
    Given a completed instance in the store
    When a new engine calls "Engine::handle::<W>(id)" and awaits it
    Then it returns the stored result without running anything

  @todo
  Scenario: Asking for a handle with the wrong workflow type fails
    When I call "Engine::handle::<RefundWorkflow>(id)" for an "OrderWorkflow" instance
    Then it fails with "Error::Invalid"

  @implemented
  Scenario: Status of an unknown instance is an error
    When I call "Engine::status(\"does-not-exist\")"
    Then it fails with an error that says the instance was not found

  @todo
  Scenario: Instances can be listed and filtered
    Given these instances in the store:
      | id  | workflow           | status    | started              |
      | o-1 | shop/order@1.0.0   | Running   | 2026-01-01T00:00:00Z |
      | o-2 | shop/order@1.0.0   | Completed | 2026-01-02T00:00:00Z |
      | r-1 | shop/refund@1.0.0  | Faulted   | 2026-01-03T00:00:00Z |
    When I call "Engine::list" with status "Running"
    Then I get "o-1"
    When I call "Engine::list" with workflow "shop/order"
    Then I get "o-1" and "o-2"
    When I call "Engine::list" with started after "2026-01-02T00:00:00Z"
    Then I get "o-2" and "r-1"

  @todo
  Scenario: Listing is paginated
    Given 250 instances in the store
    When I list them with a page size of 100
    Then I get 3 pages with 100, 100 and 50 instances
    And no instance appears twice

  @todo
  Scenario: Starting an instance does not rewrite an index of all instances
    Given 10000 instances in the store
    When I start one more
    Then the store write for the start is independent of the number of existing instances

  @implemented
  Scenario: Cancelling an instance stops it
    Given a running instance waiting on a 1-hour sleep
    When I call "cancel" on its handle
    Then awaiting the handle returns "Error::Cancelled"
    And its status is "Cancelled"
    And the "WorkflowCancelled" event was emitted

  @todo
  Scenario: Cancellation survives a restart
    Given a running instance that was cancelled
    When a new engine recovers the store
    Then the instance is not resumed
    And its status is "Cancelled"

  @todo
  Scenario: Cancelling a parent cancels its children
    Given a parent waiting on a running child
    When the parent is cancelled
    Then the child's status becomes "Cancelled"

  @todo
  Scenario: Finished instances are purged after the retention period
    Given an engine built with "retention(7.days())"
    And a completed instance that finished 8 days ago
    And a completed instance that finished 1 day ago
    And a running instance that started 30 days ago
    When the retention sweep runs
    Then the 8-day-old instance and its history are removed from the store
    And the other two instances remain

  @todo
  Scenario: An instance can be purged on demand
    Given a completed instance
    When I call "Engine::purge(id)"
    Then "Engine::status(id)" reports the instance as not found

  @todo
  Scenario: A running instance cannot be purged
    Given a running instance
    When I call "Engine::purge(id)"
    Then it fails with an error that says the instance is still running
