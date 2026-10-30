@steps @signals
Feature: Workflow state driven by signals
  As a Rust developer
  I want signals to change workflow state and wake the run method
  So that long-lived workflows can be steered while they run

  Background:
    Given an engine with a Redb store
    And a workflow "Cart" whose state is a list of items and a "checked_out" flag
    And "#[signal] add_item", "#[signal] remove_item" and "#[signal] checkout" handlers
    And a "#[query] items" handler
    And a run method that does:
      """rust
      ctx.wait_condition(|cart| cart.checked_out).await?;
      let items = ctx.state(|cart| cart.items.clone());
      Ok(ctx.call(PlaceOrder, &items).await?)
      """

  @implemented
  Scenario: Signals change state that the run method reads
    Given a running "Cart"
    When I signal "add_item" with "A", "add_item" with "B" and "remove_item" with "A"
    And I signal "checkout"
    Then "place_order" ran with ["B"]

  @implemented
  Scenario: Queries see the latest state
    Given a running "Cart" that received "add_item" with "A"
    When I query "items"
    Then the answer is ["A"]

  @implemented
  Scenario: #[init] builds the initial state
    Given "Cart" has an "#[init]" that starts with the item "welcome-gift"
    When I execute it and query "items"
    Then the answer is ["welcome-gift"]

  @implemented @crash
  Scenario: Signals are re-applied in order when the instance is rebuilt
    Given a running "Cart" that received "add_item" A, "add_item" B, "remove_item" A
    When the engine is dropped and a new engine resumes the instance
    And I query "items"
    Then the answer is ["B"]

  @implemented
  Scenario: wait_condition can be raced against a timer
    Given a run method that does "jackdaw::select! { _ = ctx.wait_condition(|c| c.checked_out) => .., _ = ctx.sleep(7.days()) => abandon }"
    When 7 days pass without "checkout"
    Then the workflow takes the abandon branch

  @implemented
  Scenario: A signal handler cannot run steps
    Given a "#[signal]" handler that tries to call "ctx.call"
    When I compile it
    Then it fails because "SyncWorkflowContext" has no "call"

  @todo
  Scenario: An update validates and answers the caller
    Given a "#[update] add_item(&mut self, ctx, sku: String) -> Result<usize, UpdateError>" that rejects unknown skus
    When I send the update with a known sku
    Then the caller receives the new item count
    When I send the update with an unknown sku
    Then the caller receives the rejection and the state is unchanged
