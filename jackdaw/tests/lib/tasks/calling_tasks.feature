@steps @call
Feature: Calling tasks from a workflow
  As a Rust developer
  I want "ctx.call" to run a task as a recorded step
  So that each side effect happens once, even across restarts

  Background:
    Given an engine with a Redb store
    And the tasks:
      """rust
      #[task]
      async fn reserve(sku: String) -> Result<u32, TaskError> { /* counts invocations */ }

      #[task]
      async fn charge(ctx: TaskContext, cents: u64) -> Result<String, TaskError> { /* records ctx */ }
      """

  @implemented
  Scenario: A step returns the task's typed output
    Given a workflow whose run does:
      """rust
      let qty = ctx.call(Reserve, &"A-1".to_string()).await?;
      Ok(qty)
      """
    When I execute it
    Then the handle resolves to the "u32" the task returned

  @implemented
  Scenario: Steps run in program order and each output feeds the next
    Given a workflow whose run does:
      """rust
      let qty = ctx.call(Reserve, &sku).await?;
      let charge_id = ctx.call(Charge, &(u64::from(qty) * 500)).await?;
      Ok(charge_id)
      """
    When I execute it
    Then "reserve" started before "charge"
    And "charge" received the cents computed from "reserve"'s output

  @implemented
  Scenario: A task can receive its TaskContext
    Given a workflow that calls "Charge"
    When I execute it
    Then the task's context reports the instance id, the task name "charge" and attempt 1
    And a non-empty idempotency key

  @implemented
  Scenario: The same task can be called many times in one workflow
    Given a workflow that calls "Reserve" for "A-1", then "B-2", then "A-1" again
    When I execute it
    Then "reserve" ran 3 times
    And the journal holds 3 distinct steps

  @implemented
  Scenario: A completed workflow is replayed without running its tasks
    Given a completed instance that called "Reserve" and "Charge"
    When a new engine on the same store replays it with "Engine::replay_all"
    Then neither task runs
    And no divergence is reported

  @implemented @crash
  Scenario: Tasks that finished before a crash are not run again
    Given a workflow that calls "Reserve", then "Charge", then a task that blocks until released
    When the engine is dropped while the blocking task runs
    And a new engine resumes the instance
    Then "reserve" and "charge" each ran once in total
    And the blocking task runs again

  @implemented
  Scenario: Calling an unregistered task is caught when the engine is built
    Given a workflow that calls "Charge"
    When I build an engine that registers the workflow but not "Charge"
    Then "build" fails with "Error::Invalid" naming "charge"

  @implemented
  Scenario: Methods of a #[tasks] type share state across steps
    Given a "#[tasks]" type "Inventory" holding a shared stock map
    And a workflow that calls "Inventory::reserve" twice for the same sku
    When I execute it
    Then the shared stock map shows both reservations

  @implemented
  Scenario: A blocking task does not stall other workflows
    Given a "#[task(blocking)]" function that sleeps its thread for 300 ms
    And a second workflow that only waits on a 10 ms step
    When I execute both at the same time
    Then the second workflow completes first

  @implemented
  Scenario: Per-step options override the task's defaults
    Given a workflow that calls the same task once with ".retry(Retry::exponential(5))" and once with no options
    When the task fails retryably on its first attempt both times
    Then the first step succeeds on attempt 2
    And the second step fails after 1 attempt

  @implemented
  Scenario: A single step can opt into caching
    Given two instances that call "Reserve" with the same input using ".cached()"
    When both run
    Then "reserve" ran once
