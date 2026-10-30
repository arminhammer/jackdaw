@steps @loops
Feature: Loops in Rust workflows
  As a Rust developer
  I want to loop over steps with for, while and loop
  So that batches and polling are plain Rust

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: A for loop runs a step per item in order
    Given a workflow whose run does:
      """rust
      let mut prices = Vec::new();
      for line in &order.lines {
          prices.push(ctx.call(Price, line).await?);
      }
      Ok(prices)
      """
    When I execute it with 4 lines
    Then "price" ran 4 times in line order
    And the result has 4 prices in line order

  @implemented
  Scenario: A while loop polls until a condition holds
    Given a workflow whose run does:
      """rust
      let mut status = ctx.call(CheckExport, &job).await?;
      while status != Status::Done {
          ctx.sleep(30.secs()).await?;
          status = ctx.call(CheckExport, &job).await?;
      }
      Ok(status)
      """
    And "CheckExport" returns "Running" twice and then "Done"
    When I execute it on a virtual clock
    Then "check_export" ran 3 times
    And 60 seconds of virtual time passed

  @implemented
  Scenario: break and continue work across steps
    Given a workflow that loops over 10 items, skips odd items with "continue", and breaks when a step returns "stop"
    And the step returns "stop" for item 6
    When I execute it
    Then the step ran for items 0, 2, 4 and 6 only

  @implemented
  Scenario: ? inside a loop stops the workflow at the failing iteration
    Given a "for" loop over 5 items whose step fails non-retryably on item 3
    When I execute it
    Then the workflow fails with that fault
    And items 4 and 5 were never processed

  @implemented
  Scenario: An error can be collected instead of stopping the loop
    Given a loop that does "match ctx.call(Import, &row).await { Ok(_) => ok += 1, Err(e) => failed.push(e.to_string()) }"
    And rows 2 and 4 of 5 fail
    When I execute it
    Then the result reports 3 imported and 2 failed

  @implemented
  Scenario: ctx.for_each runs a task over a list with a concurrency limit
    Given a task that records how many copies of it run at once
    When a workflow calls "ctx.for_each(&items, Task).concurrency(4)" over 20 items
    Then 20 results come back in input order
    And at most 4 copies ran at once

  @implemented
  Scenario: ctx.for_each runs one at a time by default
    When a workflow calls "ctx.for_each(&items, Task)" over 5 items
    Then at most 1 copy ran at once

  @implemented @crash
  Scenario: A loop resumes at the iteration it was in
    Given a "for" loop over 100 items that calls a counting task
    When the engine is dropped during iteration 61
    And a new engine resumes the instance
    Then the workflow completes
    And the counting task ran exactly once for each item

  @implemented @crash
  Scenario: ctx.for_each resumes with only the items that had not finished
    Given "ctx.for_each(&items, Task).concurrency(5)" over 50 items
    When the engine is dropped after 23 items finished
    And a new engine resumes the instance
    Then each item's task ran exactly once in total

  @todo @performance
  Scenario: A long loop keeps journal writes linear
    Given a loop of 10000 iterations with one step each
    When I execute it with a store that records bytes written
    Then the total bytes written grow linearly with the iteration count

  @todo
  Scenario: An unbounded loop continues as new to bound its history
    Given a "loop" that processes one page per iteration and calls "ctx.continue_as_new(cursor)" every 500 pages
    When it processes 2000 pages
    Then no single run's journal holds more than 500 iterations
    And all runs share the instance id
