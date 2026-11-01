@durability @journal
Feature: The journal is written efficiently and failures are not hidden
  As a library user
  I want journal writes to be cheap and reliable
  So that long workflows stay fast and durability is never silently lost

  @implemented
  Scenario: A step appends to the journal instead of rewriting it
    Given an engine with a store that records the size of every write
    When a workflow runs 1000 task steps
    Then each step's write is no larger than that step's own record plus a constant overhead
    And the total bytes written grow linearly with the number of steps

  @implemented
  Scenario: A failed journal write fails the step
    Given an engine with a store that fails every write after the first 2
    When a workflow runs 5 task steps
    Then the third step fails with "Error::Persistence"
    And the instance does not report "Completed"
    And the failure is visible on the handle and in "Engine::status"

  @implemented
  Scenario: A failed event write does not lose the event silently
    Given an engine with a store that fails event writes
    When a workflow emits events
    Then the run fails with "Error::Persistence" or the engine retries the write until it succeeds
    And a warning alone is never the only outcome

  @implemented
  Scenario: Journaled payloads go through a codec
    Given an engine built with a codec that encrypts payloads with a test key
    When a workflow runs a task with input "secret-card-number"
    Then the raw store does not contain "secret-card-number"
    And a new engine with the same codec resumes and replays the instance
    And a new engine without the codec fails to read it with a clear error

  @implemented
  Scenario: A compression codec round-trips large payloads
    Given an engine built with a compressing codec
    When a task returns a 5 MiB JSON value
    Then the stored record is smaller than 5 MiB
    And the step's output after a resume equals the original value

  @implemented
  Scenario: The journal format is versioned
    Given a journal written by the previous minor release of jackdaw
    When the current release resumes it
    Then the instance completes
    And the store records which journal format version each instance uses
