Feature: Nested Workflow Execution
  As a workflow author
  I want to call workflows from within other workflows
  So that I can modularize and reuse workflow logic

  Scenario: Execute 3-level nested workflow hierarchy
    Given the following workflows are registered:
      | namespace | name       | version | file                   |
      | test      | workflow-a | 1.0.0   | workflow-a.yaml        |
      | test      | workflow-b | 1.0.0   | workflow-b.yaml        |
      | test      | workflow-c | 1.0.0   | workflow-c.yaml        |

    When I execute workflow "test/workflow-a/1.0.0" with input:
      """json
      {
        "value": 10
      }
      """

    Then the workflow should complete successfully
    And the workflow output should be:
      """json
      {
        "value": 20
      }
      """

  Scenario: Execute nested workflow with data transformation
    Given the following workflows are registered:
      | namespace | name       | version | file                   |
      | test      | workflow-b | 1.0.0   | workflow-b.yaml        |
      | test      | workflow-c | 1.0.0   | workflow-c.yaml        |

    When I execute workflow "test/workflow-b/1.0.0" with input:
      """json
      {
        "value": 15
      }
      """

    Then the workflow should complete successfully
    And the workflow output should be:
      """json
      {
        "value": 20
      }
      """

  Scenario: Execute simple workflow without nesting
    Given the following workflows are registered:
      | namespace | name       | version | file                   |
      | test      | workflow-c | 1.0.0   | workflow-c.yaml        |

    When I execute workflow "test/workflow-c/1.0.0" with input:
      """json
      {
        "value": 30
      }
      """

    Then the workflow should complete successfully
    And the workflow output should be:
      """json
      {
        "value": 20
      }
      """
