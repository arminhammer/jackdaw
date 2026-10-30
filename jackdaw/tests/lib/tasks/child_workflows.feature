@steps @children
Feature: Child workflows
  As a Rust developer
  I want one workflow to run others as steps
  So that I can compose large processes from small ones

  Background:
    Given an engine with a Redb store
    And the workflows "Order", "Payment" and "Shipping" are registered

  @implemented
  Scenario: A parent runs a child and gets its typed output
    Given "Order" does "let receipt = ctx.run(Payment::run, &charge).await?"
    When I execute "Order"
    Then "Payment" runs as its own instance
    And "Order" receives the child's "Receipt"

  @implemented
  Scenario: Children can run in parallel
    Given "Order" does "futures::try_join!(ctx.run(Payment::run, &p), ctx.run(Shipping::run, &s))"
    When I execute "Order"
    Then both children run at the same time
    And "Order" receives both outputs

  @implemented
  Scenario: Children can be fanned out over a list
    Given "Order" runs one "Shipping" child per parcel with "try_join_all"
    When I execute it with 3 parcels
    Then 3 "Shipping" instances ran

  @implemented
  Scenario: A child's failure surfaces in the parent
    Given "Payment" fails with a "payment" fault with status 402
    When "Order" runs it with "?"
    Then "Order" fails with the same fault

  @implemented
  Scenario: A child's failure can be handled
    Given "Payment" fails and "Order" matches the error and runs "Refund" instead
    When I execute "Order"
    Then "Order" completes through the "Refund" path

  @implemented
  Scenario: A child workflow is registered with its parent
    Given "Order" runs "Payment" and only "Order" is registered
    When I build the engine
    Then the engine builds and runs "Payment" as a child of "Order"

  @implemented
  Scenario: A child can be a YAML or catalog workflow
    Given "Order" does "ctx.run(&yaml_workflow, &input)" with a typed "Workflow<In, Out>"
    When I execute "Order"
    Then the YAML workflow runs as a child and its output is typed

  @implemented
  Scenario: Child instance ids are deterministic
    Given a completed "Order" instance
    When I replay it
    Then the replay refers to the same child instance ids

  @implemented @crash
  Scenario: A crash during a child resumes both
    Given "Order" is waiting on a running "Payment" whose first step completed
    When the engine is dropped and a new engine recovers
    Then "Payment" resumes without repeating its first step
    And "Order" receives its output
    And no second "Payment" instance is started

  @implemented
  Scenario: Cancelling the parent cancels its children
    Given "Order" is waiting on a running "Shipping"
    When "Order" is cancelled
    Then "Shipping" ends "Cancelled"

  @implemented
  Scenario: A child can be started without waiting for it
    Given "Order" starts "Shipping" with "ctx.start(Shipping::run, &s)" and finishes
    Then "Order" completes before "Shipping"
    And "Shipping" keeps running with its own handle
