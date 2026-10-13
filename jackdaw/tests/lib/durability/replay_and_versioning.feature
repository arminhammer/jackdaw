@durability @versioning
Feature: Changing a workflow while instances are running
  As a library user
  I want to know before deploying whether a code change breaks running instances
  So that I can patch the workflow instead of corrupting them

  @implemented
  Scenario: replay_all reports no divergence for an unchanged workflow
    Given a store with 10 completed and 5 running instances of a workflow
    When an engine with the same code calls "Engine::replay_all"
    Then it returns an empty list

  @implemented
  Scenario: replay_all reports a divergence for an incompatible change
    Given a store with running instances of a workflow that calls "A" then "B"
    When an engine whose code calls "A" then "C" calls "Engine::replay_all"
    Then it returns one divergence per running instance
    And each divergence names the step, what the journal recorded and what the code did

  @implemented
  Scenario: patched() lets old and new instances take different paths
    Given a running instance that started before a change
    And the new code wraps the change in "if ctx.patched(\"use-new-carrier\")"
    When the instance resumes on the new code
    Then "patched(\"use-new-carrier\")" is false for it
    And an instance started on the new code sees true
    And "Engine::replay_all" reports no divergence for either

  @implemented
  Scenario: Strict replay fails a resumed instance that diverges
    Given a running instance whose journal does not match the current code
    When it is resumed
    Then it fails with "Error::NonDeterminism" carrying the divergence
    And it does not run any step past the divergence

  @todo
  Scenario: Workflow versions are side by side
    Given an engine that registers "shop/order@1.0.0" and "shop/order@2.0.0"
    When "Engine::recover" resumes instances of both versions
    Then each instance runs on the code of the version that started it
    And new executions start on the version the caller names
