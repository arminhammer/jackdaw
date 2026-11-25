@durability @redb
Feature: A Redb store written by an earlier release keeps opening
  As a library user
  I want my existing Redb file to keep working when I upgrade jackdaw
  So that running workflows and cached results survive the upgrade

  The files in `tests/fixtures/redb4` were written by jackdaw 0.3.0, which used redb 4 (see the generator there).

  @implemented
  Scenario Outline: A database written on redb 4 opens, with everything in it
    Given the Redb database written by jackdaw 0.3.0 on redb 4 and <state>
    When the current release opens it
    Then the history, the lease and the message it holds are all there
    And its waiting instance resumes, takes its signal and completes without running its first step again
    And a new instance with the same input gets the first step's result from the cache

    Examples:
      | state                          |
      | closed cleanly                 |
      | left open by a killed process  |
