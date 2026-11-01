@interop @catalogs
Feature: Typed catalog imports
  As a library user
  I want to import tasks and workflows from a catalog with types
  So that shared building blocks are checked like my own code

  @implemented
  Scenario: A catalog task is called with a typed input
    Given a "#[catalog]" that imports the function "add-numbers@1.0.0" with input "Add" and output "Sum"
    And an engine with the catalog registered
    When a workflow calls it with 2 and 3
    Then the step returns a sum of 5

  @implemented
  Scenario: A catalog workflow can be executed directly
    Given a "#[catalog]" that imports a workflow
    When I call "Engine::execute" with the catalog workflow marker
    Then the catalog's workflow definition runs

  @implemented
  Scenario: A catalog entry that does not exist fails the build
    Given a "#[catalog]" that imports "missing@1.0.0"
    When I build an engine with it
    Then "build" fails with "Error::Invalid" naming "missing:1.0.0"
