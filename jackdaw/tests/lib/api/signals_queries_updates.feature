@api @messages
Feature: Signals, queries and updates
  As a library user
  I want to talk to a running workflow
  So that people and other systems can steer it

  Background:
    Given an engine with a Redb store
    And the "Approval" workflow whose state has an "approved" flag
    And a "#[signal] approve" handler that sets the flag
    And a "#[query] is_approved" handler that returns the flag
    And a "#[run]" body that waits with "ctx.wait_condition(|s| s.approved)"

  @implemented
  Scenario: A signal changes the workflow's state
    Given a running "Approval" instance
    When I send the signal "approve" through its handle
    Then the instance completes

  @implemented
  Scenario: A query reads state without changing it
    Given a running "Approval" instance
    When I query "is_approved"
    Then the answer is false
    And the instance's journal has no new entries

  @implemented
  Scenario: Signal and query arguments are typed
    When I send "approve" with an argument of the wrong type
    Then it fails to compile

  @implemented
  Scenario: A delivered signal survives a restart
    Given a running "Approval" instance
    When I send the signal "approve"
    And the engine is dropped before the instance takes its next step
    And a new engine on the same store resumes the instance
    Then the instance completes without the signal being sent again

  @implemented
  Scenario: A signal to a finished instance is an error
    Given a completed "Approval" instance
    When I send it the signal "approve"
    Then the call fails with an error that says the instance is not running

  @implemented
  Scenario: An update returns a typed result to the caller
    Given a "#[update] set_limit(&mut self, ctx, limit: u64) -> Result<u64, UpdateError>" handler that returns the previous limit
    And a running instance with limit 10
    When I send the update "set_limit" with 20
    Then the caller receives 10
    And a query of the limit returns 20

  @implemented
  Scenario: An update can reject its input without changing state
    Given an update handler that rejects limits above 100
    When I send the update "set_limit" with 500
    Then the caller receives the rejection
    And the workflow's state is unchanged
    And the journal records no update

  @implemented
  Scenario: An accepted update is replayed, not re-applied
    Given an instance that accepted the update "set_limit" with 20
    When the instance is replayed from its journal
    Then the update handler runs once in the replay at the same point
    And the caller is not sent a second result

  @implemented
  Scenario: An update is idempotent by update id
    When I send the update "set_limit" with 20 and update id "u-1" twice
    Then the handler runs once
    And both calls receive the same result
