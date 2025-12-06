Feature: Run Script Examples
  As a user of the workflow DSL
  I want to validate that the run script examples work correctly
  So that I can use them as reference implementations

  Scenario: Run Script with stdin and arguments
    Given the example workflow "run-script-with-stdin-and-arguments.yaml"
    When the workflow is executed
    Then the workflow should complete
    And the workflow output should contain stdout with "Hello Workflow"
    And the workflow output should contain stdout with "arg"
    And the workflow output should contain stdout with "hello"
    And the workflow output should contain stdout with "env"
    And the workflow output should contain stdout with "bar"
