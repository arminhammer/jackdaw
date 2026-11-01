@operations @observability
Feature: Tracing and metrics
  As an operator
  I want spans and metrics for workflows and steps
  So that I can see what the engine is doing in production

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: Each instance and step has a tracing span
    Given a tracing subscriber that records spans
    When a workflow with two task steps runs
    Then there is a span "workflow" with fields "workflow.id", "instance.id" and "replay"
    And inside it a span "step" for each step with fields "step.name", "step.kind" and "attempt"

  @implemented
  Scenario: Task code runs inside its step span
    Given a task that logs "charging" with "tracing::info!"
    When it runs in a workflow
    Then the log record is inside the step span for that task

  @implemented
  Scenario: Replayed steps do not produce duplicate logs
    Given a task that logs "charging"
    When the instance is replayed
    Then "charging" is logged once in total

  @implemented
  Scenario: Metrics are exported behind a feature
    Given the "metrics" feature is enabled and a metrics recorder is installed
    When 3 workflows complete and 1 fails
    Then the counter "jackdaw_workflows_completed_total" is 3
    And the counter "jackdaw_workflows_failed_total" is 1
    And the histogram "jackdaw_step_duration_seconds" has samples labelled by step name

  # OpenTelemetry context follows span parentage (through `tracing-opentelemetry`), so the library needs no otel feature.
  @implemented
  Scenario: Child workflow spans nest under the parent's step span
    Given a tracing subscriber that records spans
    When a parent workflow runs a child workflow
    Then the child's workflow span has the parent's step span as its parent

  @implemented
  Scenario: The library never installs a global subscriber
    When I build an engine
    Then no global tracing subscriber or metrics recorder has been installed
