@api @tasks
Feature: Tasks
  As a library user
  I want to write side effects as typed async functions
  So that the engine can record, retry and time them out for me

  Background:
    Given an in-memory engine

  @implemented
  Scenario: A task is a typed async function
    Given the task:
      """
      #[task]
      async fn double(n: u64) -> Result<u64, TaskError> { Ok(n * 2) }
      """
    When a workflow calls "Double" with 21
    Then the step returns 42

  @implemented
  Scenario: A task's name defaults to its function name and can be overridden
    Given the task "charge" declared with "#[task(name = \"payments.charge\")]"
    Then it is registered as "payments.charge"
    And a YAML workflow can "call: payments.charge"

  @implemented
  Scenario: Tasks can share state as methods of a type
    Given a type "Shop" with a shared counter and "#[tasks]" on its impl
    And two "#[task]" methods that each increment the counter
    When a workflow calls both methods
    Then the counter is 2

  @implemented
  Scenario: A blocking task runs off the async runtime
    Given a synchronous function marked "#[task(blocking)]" that sleeps the thread for 200 ms
    When a workflow calls it while another workflow is waiting on a 10 ms timer
    Then the other workflow's timer fires before the blocking task finishes

  @implemented
  Scenario: Marking an async function as blocking is a compile error
    Given "#[task(blocking)]" on an "async fn"
    When I compile it
    Then it fails with "`#[task(blocking)]` is for synchronous functions"

  @implemented
  Scenario: A step's input must match the task's input type
    Given a workflow that passes a "String" to a task that takes "u64"
    When I compile it
    Then it fails to compile with a type mismatch on the call

  @implemented
  Scenario: A retryable failure is retried
    Given a task that fails with "TaskError::retryable" on its first 2 attempts
    When a workflow calls it with "Retry::exponential(5)"
    Then the step succeeds on attempt 3
    And the event stream contains 2 "TaskRetried" events
    And "TaskContext::attempt" was 1, 2 and 3 on the three attempts

  @implemented
  Scenario: A non-retryable failure is not retried
    Given a task that fails with "TaskError::non_retryable"
    When a workflow calls it with "Retry::exponential(5)"
    Then the step fails after 1 attempt
    And the workflow can handle the failure with "?" or "match"

  @implemented
  Scenario: Retries stop at the maximum number of attempts
    Given a task that always fails with "TaskError::retryable"
    When a workflow calls it with "Retry::constant(1.millis()).max_attempts(3)"
    Then the step fails after 3 attempts

  @implemented
  Scenario: A task can ask for a specific delay before its next retry
    Given a task that fails with "TaskError::retryable(..).next_retry_delay(5.secs())"
    When a workflow calls it with a retry policy on a virtual clock
    Then the next attempt starts 5 seconds later

  @implemented
  Scenario: A step that runs too long times out
    Given a task that takes 10 seconds
    When a workflow calls it with ".timeout(1.secs())"
    Then the step fails with "StepError::Timeout"

  @implemented
  Scenario: A panicking task becomes a fault, not a crash
    Given a task that panics with "boom"
    When a workflow calls it
    Then the step fails with a fault whose detail contains "boom"
    And the engine keeps running other workflows

  @implemented
  Scenario: The idempotency key is stable across retries and replay
    Given a task that records "TaskContext::idempotency_key" on every attempt and fails the first one
    When a workflow calls it with a retry policy
    And the instance is replayed from its journal
    Then every recorded key is the same

  @implemented
  Scenario: A cached task reuses results for equal input
    Given a task declared with "#[task(cached)]" that counts its invocations
    When two separate workflow instances call it with the same input
    Then the task ran once

  @implemented
  Scenario: Tasks are not cached by default
    Given a task declared with "#[task]" that counts its invocations
    When two separate workflow instances call it with the same input
    Then the task ran twice

  @todo
  Scenario: A single step can opt into the cache
    Given a task declared with "#[task]" that counts its invocations
    When two separate workflow instances call it with the same input using ".cached()"
    Then the task ran once

  @todo
  Scenario: A cached step can bypass the cache
    Given a task declared with "#[task(cached)]"
    When a workflow calls it with ".cached(false)"
    Then the task runs even though an equal input was cached

  @implemented
  Scenario: A task can run on its own with step semantics
    When I call "Engine::run_task(Double, 21, TaskOptions::new().retry(Retry::exponential(3)))"
    Then it returns 42
    And no workflow instance is recorded in the store

  @todo
  Scenario: A long task heartbeats its progress
    Given a task that calls "TaskContext::heartbeat(progress)" every second
    And the step has ".heartbeat_timeout(3.secs())"
    When the task stops heartbeating for 3 seconds
    Then the attempt fails with a heartbeat timeout and is retried
    And the next attempt can read the last progress with "TaskContext::last_heartbeat"

  @todo
  Scenario: A running task observes cancellation
    Given a task that loops until "TaskContext::cancelled()" resolves
    When the instance is cancelled while the task runs
    Then the task's cancellation future resolves
    And the instance ends with status "Cancelled"
