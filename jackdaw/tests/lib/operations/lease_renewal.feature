@operations @workers @lease-renewal
Feature: Reporting lease renewal that runs late
  As an operator running several engines on one store
  I want to be told when an engine could not renew its leases on time
  So that I find the code that blocks the runtime before another engine takes its work over

  # Each scenario runs its engine on a runtime of its own with one thread, so code that blocks that thread
  # stops the engine's background work for as long as it runs.
  @implemented
  Scenario: Lease renewal held up by an async task that blocks the runtime is reported
    Given an engine with a lease of 1 second on a runtime with one thread
    When a workflow on it calls an async task that blocks the thread for 2 seconds
    Then a warning says lease renewal ran late
    And the warning suggests "#[task(blocking)]"

  @implemented
  Scenario: Blocking tasks do not hold up lease renewal
    Given an engine with a lease of 1 second on a runtime with one thread
    When a workflow on it calls a "#[task(blocking)]" task that sleeps for 2 seconds
    Then no warning about late lease renewal is recorded
