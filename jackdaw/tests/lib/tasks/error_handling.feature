@steps @errors
Feature: Handling errors in Rust workflows
  As a Rust developer
  I want step failures as ordinary Result values
  So that I can retry, recover, compensate or fail with ? and match

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: ? propagates a step's failure as the workflow's failure
    Given a workflow that does "ctx.call(Charge, &cents).await?" and "Charge" fails with "TaskError::non_retryable(\"card expired\")"
    When I execute it
    Then awaiting the handle fails with "Error::Faulted" whose detail contains "card expired"
    And "Engine::status" is "Faulted"

  @implemented
  Scenario: A specific fault can be handled and the workflow continues
    Given a workflow whose run does:
      """rust
      match ctx.call(Charge, &cents).retry(Retry::exponential(5)).await {
          Ok(id) => Ok(Outcome::Charged(id)),
          Err(StepError::Fault(fault)) if fault.status == Some(402) => {
              ctx.call(Notify, &"card declined".to_string()).await?;
              Ok(Outcome::Declined)
          }
          Err(other) => Err(other),
      }
      """
    When "Charge" fails with "TaskError::fault(\"payment\").status(402)"
    Then the workflow completes with "Declined"
    And "notify" ran once

  @implemented
  Scenario: Retryable errors are retried by the step's policy
    Given "Charge" fails with "TaskError::retryable" on attempts 1 and 2
    When a workflow calls it with ".retry(Retry::exponential(5))"
    Then the step succeeds on attempt 3
    And the events include 2 "TaskRetried" events

  @implemented
  Scenario: Non-retryable errors skip the retry policy
    Given "Charge" fails with "TaskError::non_retryable"
    When a workflow calls it with ".retry(Retry::exponential(5))"
    Then the step fails after 1 attempt

  @implemented
  Scenario: A step timeout is its own error
    Given a task that takes 10 seconds
    When a workflow calls it with ".timeout(1.secs())" and matches the result
    Then the result is "Err(StepError::Timeout { .. })"

  @implemented
  Scenario: A panicking task is a fault, not a crash
    Given a task that panics with "index out of bounds"
    When a workflow calls it
    Then the step fails with a runtime fault whose detail contains "index out of bounds"
    And other running instances are unaffected

  @implemented
  Scenario: A workflow can fail with its own fault
    Given a workflow that returns "Err(TaskError::fault(\"validation\").status(400).title(\"Empty order\").into())"
    When I execute it
    Then awaiting the handle fails with a validation fault with status 400 and title "Empty order"

  @implemented
  Scenario: Compensation undoes earlier steps when a later one fails
    Given a workflow whose run does:
      """rust
      let hold = ctx.call(ReserveStock, &order).await?;
      let charge = match ctx.call(Charge, &order).await {
          Ok(c) => c,
          Err(e) => {
              ctx.call(ReleaseStock, &hold).await?;
              return Err(e);
          }
      };
      Ok(charge)
      """
    When "Charge" fails non-retryably
    Then "release_stock" ran once with the hold from "reserve_stock"
    And the workflow fails with "Charge"'s fault

  @implemented @crash
  Scenario: Compensation that was interrupted resumes and completes
    Given the compensation step "ReleaseStock" was running when the engine was dropped
    When a new engine resumes the instance
    Then "release_stock" completes
    And "reserve_stock" and "charge" are not run again

  @implemented
  Scenario: Retry attempts are replayed, not repeated
    Given a completed instance whose step succeeded on attempt 3
    When I replay it
    Then the task does not run
    And no divergence is reported

  @implemented
  Scenario: Compensation runs even when the workflow is cancelled
    Given a workflow that registers compensations for each step it completes
    When it is cancelled after two steps
    Then the two compensations run in reverse order in a non-cancellable scope
    And the instance ends "Cancelled"
