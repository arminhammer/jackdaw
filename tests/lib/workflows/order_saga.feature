@workflows @saga
Feature: Order fulfillment saga
  As a Rust developer building a shop
  I want an order workflow that charges, reserves and ships with compensation
  So that a failure part-way through leaves no money taken and no stock held

  Background:
    Given an engine with a Redb store and a controllable clock
    And the tasks "ReserveStock", "ReleaseStock", "Charge", "Refund", "CreateShipment", "CancelShipment" and "Notify" that count their invocations
    And the workflow:
      """rust
      #[workflow(namespace = "shop", name = "order", version = "1.0.0")]
      struct OrderSaga;

      #[workflow_methods]
      impl OrderSaga {
          #[run]
          async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Receipt> {
              let hold = ctx.call(ReserveStock, &order).retry(Retry::exponential(5)).await?;

              let charge = match ctx.call(Charge, &order).retry(Retry::exponential(3)).timeout(30.secs()).await {
                  Ok(charge) => charge,
                  Err(error) => {
                      ctx.call(ReleaseStock, &hold).await?;
                      ctx.call(Notify, &Message::declined(&order)).await?;
                      return Err(error);
                  }
              };

              let shipment = match ctx.call(CreateShipment, &(order.clone(), hold.clone())).await {
                  Ok(shipment) => shipment,
                  Err(error) => {
                      futures::try_join!(ctx.call(Refund, &charge), ctx.call(ReleaseStock, &hold))?;
                      return Err(error);
                  }
              };

              ctx.call(Notify, &Message::shipped(&order, &shipment)).await?;
              Ok(Receipt { charge, shipment })
          }
      }
      """

  @implemented
  Scenario: The happy path charges, ships and notifies once
    When I execute "OrderSaga" for an order of 2 items
    Then the workflow completes with a receipt
    And these tasks ran exactly once: ReserveStock, Charge, CreateShipment, Notify
    And these tasks never ran: ReleaseStock, Refund, CancelShipment

  @implemented
  Scenario: A transient stock error is retried
    Given "ReserveStock" fails retryably twice
    When I execute "OrderSaga"
    Then the workflow completes
    And "ReserveStock" ran 3 times

  @implemented
  Scenario: A declined card releases the stock and notifies the customer
    Given "Charge" fails with "TaskError::fault(\"payment\").status(402)"
    When I execute "OrderSaga"
    Then the workflow fails with the 402 fault
    And "ReleaseStock" ran once with the hold
    And "Notify" ran once with a "declined" message
    And "CreateShipment" never ran

  @implemented
  Scenario: A failed shipment refunds and releases in parallel
    Given "CreateShipment" fails non-retryably
    When I execute "OrderSaga"
    Then "Refund" and "ReleaseStock" each ran once and overlapped in time
    And the workflow fails with the shipment fault

  @implemented
  Scenario: A charge that times out is compensated
    Given "Charge" takes 60 seconds
    When I execute "OrderSaga"
    Then the workflow fails with a timeout
    And "ReleaseStock" ran once

  @implemented @crash
  Scenario: A crash after charging does not charge twice
    Given "CreateShipment" blocks until released
    When I execute "OrderSaga"
    And the engine is dropped while "CreateShipment" runs
    And a new engine resumes the instance and "CreateShipment" is released
    Then the workflow completes
    And "Charge" ran exactly once in total
    And "Charge" received the same idempotency key in every attempt

  @implemented @crash
  Scenario: A crash during compensation finishes the compensation
    Given "CreateShipment" fails and "Refund" blocks until released
    When the engine is dropped while "Refund" runs
    And a new engine resumes the instance and "Refund" is released
    Then "Refund" and "ReleaseStock" each completed once in total
    And "Charge" ran once in total

  @implemented
  Scenario: 50 concurrent orders do not interfere
    When I execute 50 "OrderSaga" instances at once, where every 5th card is declined
    Then 40 complete and 10 fail
    And "ReleaseStock" ran exactly 10 times
    And each completed instance's history shows its own tasks only

  @implemented
  Scenario: The saga passes its replay check
    Given 20 completed or failed "OrderSaga" instances
    When I run "Engine::replay_all"
    Then no divergence is reported
