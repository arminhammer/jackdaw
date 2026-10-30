@api @events
Feature: External events and the event stream
  As a library user
  I want to feed events into workflows and watch what they do
  So that workflows can wait on the outside world and I can observe progress

  @implemented
  Scenario: A workflow waits for a named event
    Given a workflow that calls "ctx.wait_for_event::<Payment>(\"paid\")"
    When I call "send_event(\"paid\", Payment { cents: 500 })" on its handle
    Then the step returns the payment with 500 cents

  @implemented
  Scenario: An event sent before the workflow waits is not lost
    Given a workflow that sleeps 1 second and then waits for "paid"
    When I send "paid" immediately after starting it
    Then the workflow receives the event once it starts waiting

  @todo @durability
  Scenario: An event sent before a crash is delivered after resume
    Given an engine with a Redb store
    And a workflow that sleeps 1 hour and then waits for "paid"
    When I send "paid" to the running instance
    And the engine is dropped
    And a new engine on the same store resumes the instance
    And the virtual clock advances 1 hour
    Then the workflow receives the event without it being sent again

  @todo @durability
  Scenario: An event is persisted before send_event returns
    Given an engine with a store that records every write
    When "send_event" returns successfully
    Then the store has recorded the event for that instance

  @implemented
  Scenario: The handle streams lifecycle events in order
    Given a workflow with two task steps
    When I execute it and read "ExecutionHandle::events" to the end
    Then the event names are, in order:
      | event             |
      | WorkflowStarted   |
      | TaskStarted       |
      | TaskCompleted     |
      | TaskStarted       |
      | TaskCompleted     |
      | WorkflowCompleted |
    And the stream ends with "None" after the terminal event

  @implemented
  Scenario: Replayed steps are marked as replayed
    Given a completed instance in a Redb store
    When a new engine resumes it
    Then every "TaskCompleted" event from the journal reports "replayed() == true"

  @implemented
  Scenario: Events are persisted to the history
    Given a completed instance in a Redb store
    When I call "Engine::history" with its id from a new engine
    Then I get the same events the stream delivered

  @todo @yaml
  Scenario: Rust workflows can emit events
    Given a workflow that calls "ctx.emit(CloudEvent { type: \"order.shipped\", .. })"
    When it runs
    Then the event stream contains an "Other" event of type "order.shipped"
    And a YAML workflow listening for "order.shipped" on the same engine is triggered
