@steps @events
Feature: Waiting for external events in Rust workflows
  As a Rust developer
  I want a workflow to wait for typed events
  So that outside systems can move it forward

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: A workflow receives a typed event
    Given a workflow that does "let paid: Paid = ctx.wait_for_event(\"paid\").await?"
    When I call "handle.send_event(\"paid\", Paid { cents: 500 })"
    Then the workflow continues with 500 cents

  @implemented
  Scenario: Events that arrive early are buffered
    Given a workflow that runs a 1-second task and then waits for "paid"
    When "paid" is sent while the task is still running
    Then the workflow receives it as soon as it starts waiting

  @implemented
  Scenario: Events of the same name are consumed in order
    Given a workflow that waits for "line" three times in a loop
    When "line" is sent with 1, 2 and 3
    Then the workflow receives 1, 2 and 3 in that order

  @implemented
  Scenario: Different event names are independent
    Given a workflow that waits for "shipped" and then for "paid"
    When "paid" is sent before "shipped"
    Then the workflow receives both in its own order

  @implemented
  Scenario: A payload of the wrong shape fails the step clearly
    Given a workflow that waits for "paid" as "Paid { cents: u64 }"
    When "paid" is sent with '{ "cents": "lots" }'
    Then the step fails with a validation fault naming "paid"

  @implemented
  Scenario: A recorded event is replayed from the journal
    Given a completed instance that received "paid"
    When I replay it
    Then the replay receives the same payload without the event being sent

  @implemented @crash
  Scenario: An event sent before a crash is not lost
    Given a workflow that sleeps 1 hour and then waits for "paid"
    When "paid" is sent and the engine is dropped during the sleep
    And a new engine resumes the instance
    Then the workflow receives "paid" without it being sent again

  @implemented
  Scenario: Events can be sent to an instance by id
    Given a running instance "order-42" waiting for "paid"
    When another part of the program calls "Engine::handle::<W>(\"order-42\")?.send_event(\"paid\", ..)"
    Then the instance receives it
