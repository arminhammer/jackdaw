@workflows @long-running
Feature: Subscription billing over months
  As a Rust developer building billing
  I want one workflow per subscription that bills every month until cancelled
  So that billing state lives in one durable place

  Background:
    Given an engine with a Redb store and a controllable clock
    And the workflow:
      """rust
      #[workflow(namespace = "billing", name = "subscription", version = "1.0.0")]
      struct Subscription { plan: Plan, cancelled: bool }

      #[workflow_methods]
      impl Subscription {
          #[run]
          async fn run(ctx: &mut WorkflowContext<Self>, start: Start) -> WorkflowResult<Summary> {
              let mut invoices = start.invoices_so_far;
              loop {
                  let renew = jackdaw::select! {
                      _ = ctx.wait_condition(|s| s.cancelled) => false,
                      _ = ctx.sleep(30.days()) => true,
                  };
                  if !renew { break; }
                  let plan = ctx.state(|s| s.plan.clone());
                  match ctx.call(ChargeCard, &plan).retry(Retry::exponential(4)).await {
                      Ok(invoice) => { invoices += 1; ctx.call(SendInvoice, &invoice).await?; }
                      Err(_) => {
                          ctx.call(Dunning, &plan).await?;
                          break;
                      }
                  }
                  if ctx.patched("annual-discount") && invoices % 12 == 0 {
                      ctx.call(ApplyLoyaltyCredit, &plan).await?;
                  }
              }
              ctx.call(CloseAccount, &start).await?;
              Ok(Summary { invoices })
          }

          #[signal] fn change_plan(&mut self, _: &mut SyncWorkflowContext<Self>, plan: Plan) { self.plan = plan; }
          #[signal] fn cancel(&mut self, _: &mut SyncWorkflowContext<Self>, _: ()) { self.cancelled = true; }
          #[query]  fn plan(&self, _: &WorkflowContextView) -> Plan { self.plan.clone() }
      }
      """

  @implemented
  Scenario: The subscription bills every 30 days
    When I start a subscription and 90 days pass
    Then "ChargeCard" ran 3 times
    And "SendInvoice" ran 3 times

  @implemented
  Scenario: A plan change applies from the next bill
    When I start a subscription on "basic" and 40 days pass
    And I signal "change_plan" with "pro"
    And 30 more days pass
    Then the first charge was for "basic" and the second for "pro"

  @implemented
  Scenario: Cancelling stops billing and closes the account
    When I start a subscription and 45 days pass
    And I signal "cancel"
    Then the workflow completes with 1 invoice
    And "CloseAccount" ran once
    And no further charges happen after 60 more days

  @implemented
  Scenario: A card that keeps failing goes to dunning
    Given "ChargeCard" fails every time
    When I start a subscription and 30 days pass
    Then "ChargeCard" was attempted 4 times
    And "Dunning" ran once
    And "CloseAccount" ran once

  @implemented
  Scenario: History is bounded with continue-as-new
    Given the loop ends each 24th cycle with "return ctx.continue_as_new(Start { invoices_so_far: invoices })"
    When I start a subscription and 3 years pass
    Then the instance has continued as new at least once
    And the current run's journal holds no more than 24 billing cycles
    And querying "plan" on the original instance id still works

  @implemented @crash
  Scenario: A restart in the middle of a month does not bill early or twice
    When I start a subscription and 20 days pass
    And the engine is dropped and a new engine resumes the instance
    Then no charge happens until day 30
    And exactly one charge happens on day 30

  @implemented
  Scenario: A patch for loyalty credit applies only to new subscriptions
    Given a subscription started before "annual-discount" was added
    When it resumes on the new code and reaches its 12th invoice
    Then "ApplyLoyaltyCredit" does not run for it
    And "Engine::replay_all" reports no divergence
