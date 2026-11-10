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
  Scenario: Task code runs inside its attempt's span, inside the step span
    Given a task that logs "charging" with "tracing::info!"
    When it runs in a workflow
    Then the log record is inside the "attempt 1" span of the step span for that task

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
    And its samples are labelled by outcome: 3 "ok" and 1 "fault"
    And the histogram "jackdaw_workflow_duration_seconds" has 3 "completed" and 1 "failed" sample
    And the histogram "jackdaw_persistence_operation_duration_seconds" has samples for "append_journal"

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

  # Trace viewers show `otel.name` (through `tracing-opentelemetry`), so a trace reads as what the workflow did.
  @implemented
  Scenario: Spans are named for what they do
    Given a tracing subscriber that records spans
    When a workflow with two task steps runs
    Then the workflow span is named "workflow obs/order@1"
    And the step spans are named "call obs.charge" and "call obs.ship"
    And each step span records its position, its call site and the outcome "ok"

  @implemented
  Scenario: Each attempt of a task has a span, and a retry waits in a backoff span
    Given a tracing subscriber that records spans
    When a task step fails once and succeeds on its retry
    Then its step span has the spans "attempt 1", "retry backoff" and "attempt 2" in that order
    And the span "attempt 1" is marked as an error that may be retried
    And the step span records attempt 2 and the outcome "ok"

  @implemented
  Scenario: Every event of a run is logged in the span where it happened
    Given a tracing subscriber that records spans
    When a workflow with two task steps runs
    Then the events "WorkflowStarted" and "WorkflowCompleted" are logged in the workflow span
    And the events "TaskStarted" and "TaskCompleted" of "obs.charge" are logged in its step span

  @implemented
  Scenario: A failed step and its workflow are marked as errors
    Given a tracing subscriber that records spans
    When a workflow whose task fails runs
    Then the step span has the outcome "fault" and the status "ERROR"
    And the workflow span has the outcome "failed" and the status "ERROR"

  @implemented
  Scenario: Steps replayed on resume are summed up in one replay span
    Given a task that logs "charging"
    When the instance is replayed
    Then the resumed workflow span has one span "replay 1 step" with an event for "obs.charge"
    And the live step "obs.ship" is not inside it

  @implemented
  Scenario: Step payloads are not on spans by default
    Given a tracing subscriber that records spans
    When a workflow with two task steps runs
    Then the step spans record "step.input.bytes" and "step.output.bytes" but not "step.input" or "step.output"

  @implemented
  Scenario: Step payloads are on spans, truncated, when turned on
    Given an engine built with "trace_payloads(2)"
    And a tracing subscriber that records spans
    When a workflow with two task steps runs
    Then the step span of "obs.charge" records "step.input" as "25… (3 bytes)"

  @implemented
  Scenario: A workflow started with ctx.start has a trace of its own
    Given a tracing subscriber that records spans
    When a workflow starts another with "ctx.start"
    Then the started workflow's span has no parent
    And the "start" step span records the started instance's id

  @implemented
  Scenario: The runs of an instance that continues as new are numbered
    Given a tracing subscriber that records spans
    When a workflow continues as new once
    Then its step spans record the runs 1 and 2

  @implemented
  Scenario: Attempts, retries and cache lookups have metrics
    Given the "metrics" feature is enabled and a metrics recorder is installed
    When a metered task step fails once and succeeds on its retry
    Then the histogram "jackdaw_task_attempt_duration_seconds" of "obs.wobbly" has 1 "retryable_error" and 1 "ok" sample
    And the counter "jackdaw_task_retries_total" of "obs.wobbly" is 1

  @implemented
  Scenario: A YAML workflow's tasks are spans, nested as in the workflow
    Given a tracing subscriber that records spans
    When a YAML workflow sets a list and loops over it with "for", running a "set" task each time
    Then the workflow span is named "workflow obs/yaml-loop@1.0.0" and says it ran on the "yaml" engine
    And its task spans are "set prepare" at "/do/0/prepare" and "for each" at "/do/1/each"
    And "for each" has the spans "iteration 0" and "iteration 1"
    And each iteration has a span "set paint" at "/do/1/each/do/0/paint"
    And the event "TaskStarted" of "paint" is logged in a "set paint" span
