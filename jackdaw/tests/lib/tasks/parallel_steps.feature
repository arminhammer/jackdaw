@steps @parallel
Feature: Running steps in parallel
  As a Rust developer
  I want to start several steps at once and join them
  So that independent work does not wait on each other

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: try_join runs two steps concurrently
    Given tasks "Price" and "Stock" that each take 300 ms
    And a workflow whose run does:
      """rust
      let (price, stock) = futures::try_join!(
          ctx.call(Price, &sku),
          ctx.call(Stock, &sku),
      )?;
      Ok(Quote { price, stock })
      """
    When I execute it
    Then it completes in less than 550 ms
    And the quote has both results

  @implemented
  Scenario: join_all fans out over a list and keeps order
    Given a workflow that does "futures::future::try_join_all(skus.iter().map(|s| ctx.call(Price, s))).await?"
    When I execute it with 6 skus
    Then 6 prices come back in sku order
    And the 6 calls overlapped in time

  @implemented
  Scenario: Branches can be different kinds of step
    Given a workflow that try_joins a task call, a child workflow and a 1-second sleep
    When I execute it on a virtual clock
    Then all three complete and the results are returned as a tuple

  @implemented
  Scenario: A failure in one branch fails the join
    Given a try_join of "Price", which fails non-retryably, and "Stock", which takes 1 second
    When I execute it
    Then the workflow fails with "Price"'s fault

  @implemented
  Scenario: join keeps every result, including failures
    Given a "futures::join!" of "Price", which fails, and "Stock", which succeeds
    When the workflow inspects both results
    Then it can continue with "Stock"'s value and report "Price"'s error

  @implemented
  Scenario: Replay of a parallel section is deterministic
    Given a completed instance whose two parallel steps finished in the order "Stock", "Price"
    When I replay it with the steps now finishing in the opposite order
    Then no divergence is reported

  @implemented @crash
  Scenario: Only unfinished branches run again after a crash
    Given a try_join of three counting tasks where the first two have finished
    When the engine is dropped and a new engine resumes the instance
    Then the first two tasks ran once in total
    And the third task runs to completion

  @implemented
  Scenario: Nested parallelism works
    Given a workflow that try_joins two blocks that each try_join two task calls
    When I execute it
    Then all four calls overlapped in time
