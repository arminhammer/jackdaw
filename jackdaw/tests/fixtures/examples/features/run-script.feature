Feature: Run Script Examples
  As a user of the workflow DSL
  I want to validate that the run script examples work correctly
  So that I can use them as reference implementations

  Scenario: Run Script with stdin and arguments
    Given the example workflow "run-shell-stdin-and-arguments.yaml"
    When the workflow is executed
    Then the workflow should complete
    # TODO: assert 'STDIN was: Hello World' once the SDK's ShellProcessDefinition has
    # `stdin` and `directory` and the executor supports them
    And the workflow output should contain stdout with "ARGS are Foo Bar"
