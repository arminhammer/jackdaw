@steps @limits
Feature: Parallel task calls and named limits
  As a Rust developer running batch work
  I want concurrent task calls to use every core, and a way to cap how many run at once
  So that CPU-heavy work finishes sooner without overwhelming the machine or a downstream service

  Background:
    Given an engine with a Redb store

  # Concurrent async calls of one instance each get their own tokio task. The rendezvous task blocks its thread
  # until a second call arrives, so it only completes when the two calls run on different threads.
  @implemented
  Scenario Outline: Concurrent async task calls of one workflow run in parallel
    Given an async task that blocks its thread until a second call arrives
    When a workflow calls it twice with <fan-out>
    Then both rendezvous calls complete

    Examples:
      | fan-out                                  |
      | "futures::try_join!"                     |
      | "ctx.for_each(..).concurrency(2)"        |

  @implemented
  Scenario: A named limit caps the calls running at once across workflows
    Given an async task marked "#[task(limit = \"render\")]" that holds until released
    And the engine defines the limit "render" with 2 permits
    When 5 workflows each call the held task
    Then exactly 2 held calls are running
    When the held calls are released
    Then all 5 workflows complete
    And at most 2 held calls ever ran at once

  @implemented
  Scenario: Tasks that name the same limit share its permits
    Given a free-function task and a "#[tasks]" method that both name the limit "shared" and hold until released
    And the engine defines the limit "shared" with 1 permit
    When one workflow calls the free-function task and another calls the method
    Then exactly 1 held call is running
    When the held calls are released
    Then all 2 workflows complete
    And at most 1 held call ever ran at once

  @implemented
  Scenario: The blocking limit caps blocking tasks that name no limit
    Given a "#[task(blocking)]" task that holds until released
    And the engine sets a blocking limit of 2
    When 4 workflows each call the held task
    Then exactly 2 held calls are running
    When the held calls are released
    Then all 4 workflows complete
    And at most 2 held calls ever ran at once

  @implemented
  Scenario: A blocking task that names a limit uses it instead of the blocking limit
    Given a "#[task(blocking, limit = \"cpu\")]" task that holds until released
    And the engine defines the limit "cpu" with 1 permit
    And the engine sets a blocking limit of 10
    When 3 workflows each call the held task
    Then exactly 1 held call is running
    When the held calls are released
    Then all 3 workflows complete
    And at most 1 held call ever ran at once

  @implemented
  Scenario: Without a limit, blocking calls are not capped
    Given a "#[task(blocking)]" task that holds until released
    When 4 workflows each call the held task
    Then exactly 4 held calls are running
    When the held calls are released
    Then all 4 workflows complete

  @implemented
  Scenario: The builder rejects a task that names an undefined limit
    Given an async task marked "#[task(limit = \"render\")]" that holds until released
    When I build the engine without defining the limit
    Then the build fails with "uses the limit `render`, which the engine does not define"

  @implemented
  Scenario: The builder rejects a limit without permits
    Given an async task marked "#[task(limit = \"render\")]" that holds until released
    And the engine defines the limit "render" with 0 permits
    When I build the engine with its limits
    Then the build fails with "the limit `render` needs between 1 and"

  # The heartbeat timeout starts once the call has its permit: a queued call cannot send heartbeats.
  @implemented
  Scenario: Waiting for a permit does not count against the heartbeat timeout
    Given an async task marked "#[task(limit = \"render\")]" that holds until released
    And the engine defines the limit "render" with 1 permit
    And one workflow's call holds the only permit
    When another workflow calls the held task with a heartbeat timeout of 1 minute
    And 5 minutes pass while it waits for the permit
    And the held calls are released
    Then all 2 workflows complete
    And no held call was retried

  @implemented
  Scenario: A YAML workflow's calls of a limited Rust task wait for the limit too
    Given an async task marked "#[task(limit = \"render\")]" that holds until released
    And the engine defines the limit "render" with 1 permit
    And a YAML workflow that forks 3 branches, each calling the held task
    When I execute the forking YAML workflow
    Then exactly 1 held call is running
    When the held calls are released
    Then the YAML workflow completes
    And at most 1 held call ever ran at once
