@operations @observability
Feature: Tracing and metrics
  As an operator
  I want spans and metrics for workflows and steps
  So that I can see what the engine is doing in production

  @todo
  Scenario: Each instance and step has a tracing span
    Given a tracing subscriber that records spans
    When a workflow with two task steps runs
    Then there is a span "workflow" with fields "workflow.id", "instance.id" and "replay"
    And inside it a span "step" for each step with fields "step.name", "step.kind" and "attempt"

  @todo
  Scenario: Task code runs inside its step span
    Given a task that logs "charging" with "tracing::info!"
    When it runs in a workflow
    Then the log record is inside the step span for that task

  @todo
  Scenario: Replayed steps do not produce duplicate logs
    Given a task that logs "charging"
    When the instance is replayed
    Then "charging" is logged once in total

  @todo
  Scenario: Metrics are exported behind a feature
    Given the "metrics" feature is enabled and a metrics recorder is installed
    When 3 workflows complete and 1 fails
    Then the counter "jackdaw_workflows_completed_total" is 3
    And the counter "jackdaw_workflows_failed_total" is 1
    And the histogram "jackdaw_step_duration_seconds" has samples labelled by step name

  @todo
  Scenario: OpenTelemetry context propagates into child workflows
    Given the "otel" feature is enabled
    When a parent workflow runs a child workflow
    Then the child's workflow span has the parent's step span as its parent

  @todo
  Scenario: The library never installs a global subscriber
    When I build an engine
    Then no global tracing subscriber or metrics recorder has been installed
