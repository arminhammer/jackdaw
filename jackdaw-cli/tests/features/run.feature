Feature: Running workflows from the command line
  As someone who runs workflows with `jackdaw run`
  I want the result on standard output and the progress somewhere else
  So that I can pipe the result and still see what is going on

  Background:
    Given a workflow "two-steps.sw.yaml":
      """
      document:
        dsl: '1.0.2'
        name: two-steps
        namespace: examples
        version: '0.1.0'
      output:
        as: .statement
      do:
        - first:
            set:
              statement: Hello
        - second:
            set:
              statement: Hello, World!
      """

  Scenario: The result is on standard output and nothing else is
    When I run "jackdaw run two-steps.sw.yaml"
    Then it succeeds
    And standard output is exactly "Hello, World!" as JSON

  Scenario: Output that is piped gets a plain line for each step on standard error
    When I run "jackdaw run two-steps.sw.yaml"
    Then standard error has the lines:
      | first started  |
      | first done     |
      | second started |
      | second done    |

  Scenario: Quiet shows only the result
    When I run "jackdaw run two-steps.sw.yaml --quiet"
    Then it succeeds
    And standard error is empty
    And standard output is exactly "Hello, World!" as JSON

  Scenario: JSON shows one object per event
    When I run "jackdaw run two-steps.sw.yaml --json"
    Then every line of standard error is a JSON object with an "event" field
    And standard output is exactly "Hello, World!" as JSON

  Scenario: Debug shows what each step returned
    When I run "jackdaw run two-steps.sw.yaml --debug"
    Then standard error contains "first done"
    And standard error contains "\"statement\": \"Hello\""

  Scenario: A workflow that fails exits with an error and says why
    Given a workflow "boom.sw.yaml":
      """
      document:
        dsl: '1.0.2'
        name: boom
        namespace: examples
        version: '0.1.0'
      do:
        - explode:
            raise:
              error:
                type: https://example.com/errors/boom
                status: 500
                title: Boom
      """
    When I run "jackdaw run boom.sw.yaml"
    Then it fails
    And standard output is empty
    And standard error contains "explode"
    And standard error contains "Boom"

  Scenario: A run with an id that finished is not run again
    When I run "jackdaw run two-steps.sw.yaml --persistence-provider redb --durable-db run.db --id daily"
    And I run "jackdaw run two-steps.sw.yaml --persistence-provider redb --durable-db run.db --id daily"
    Then it succeeds
    And standard error contains "has finished"
    And standard output is exactly "Hello, World!" as JSON

  Scenario: A restart runs it again
    When I run "jackdaw run two-steps.sw.yaml --persistence-provider redb --durable-db run.db --id daily"
    And I run "jackdaw run two-steps.sw.yaml --persistence-provider redb --durable-db run.db --id daily --restart"
    Then it succeeds
    And standard error contains "restarting daily"
    And standard output is exactly "Hello, World!" as JSON

  Scenario: An id needs a single workflow
    Given a workflow "other.sw.yaml":
      """
      document:
        dsl: '1.0.2'
        name: other
        namespace: examples
        version: '0.1.0'
      do:
        - only:
            set:
              x: 1
      """
    When I run "jackdaw run two-steps.sw.yaml other.sw.yaml --id daily"
    Then it fails
    And standard error contains "--id"

  Scenario: Several workflows run in parallel and each prints its result
    Given a workflow "other.sw.yaml":
      """
      document:
        dsl: '1.0.2'
        name: other
        namespace: examples
        version: '0.1.0'
      output:
        as: .statement
      do:
        - only:
            set:
              statement: Goodbye
      """
    When I run "jackdaw run two-steps.sw.yaml other.sw.yaml --parallel"
    Then it succeeds
    And standard output contains "Hello, World!"
    And standard output contains "Goodbye"

  Scenario: Output of a script is streamed with the task's name
    Given a workflow "shell.sw.yaml":
      """
      document:
        dsl: '1.0.2'
        name: shell
        namespace: examples
        version: '0.1.0'
      do:
        - speak:
            run:
              shell:
                command: echo from-the-shell
      """
    When I run "jackdaw run shell.sw.yaml"
    Then it succeeds
    And standard output contains "[speak:stdout] from-the-shell"
