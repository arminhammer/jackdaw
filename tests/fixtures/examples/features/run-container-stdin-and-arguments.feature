Feature: Run Container with STDIN and Arguments
  As a user of the workflow DSL
  I want to validate that the run container examples work correctly with stdin and arguments
  So that I can use containers to execute commands with input and parameters

  Scenario: Run Container with stdin and arguments
    Given the example workflow "run-container-stdin-and-arguments.yaml"
    When the workflow is executed
    Then the workflow should complete
    And the workflow output should contain stdout with "STDIN was: Hello World"
    And the workflow output should contain stdout with "ARGS are Foo Bar"