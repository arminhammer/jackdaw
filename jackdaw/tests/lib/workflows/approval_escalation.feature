@workflows @approval
Feature: Expense approval with escalation
  As a Rust developer building an approvals service
  I want a workflow that waits for a manager, escalates and finally expires
  So that requests never get stuck and every decision is recorded

  Background:
    Given an engine with a Redb store and a controllable clock
    And the workflow:
      """rust
      #[workflow(namespace = "finance", name = "expense-approval", version = "1.0.0")]
      struct ExpenseApproval { decision: Option<Decision>, approver: Option<String> }

      #[workflow_methods]
      impl ExpenseApproval {
          #[init]
          fn new(_ctx: &WorkflowContextView) -> Self { Self { decision: None, approver: None } }

          #[run]
          async fn run(ctx: &mut WorkflowContext<Self>, expense: Expense) -> WorkflowResult<Outcome> {
              if expense.cents <= 10_000 {
                  return Ok(Outcome::AutoApproved);
              }
              let chain = ctx.call(ApproverChain, &expense).await?;   // e.g. [manager, director, cfo]
              for approver in chain {
                  ctx.call(RequestApproval, &(approver.clone(), expense.clone())).await?;
                  let decided = jackdaw::select! {
                      _ = ctx.wait_condition(|s| s.decision.is_some()) => true,
                      _ = ctx.sleep(48.hours()) => false,
                  };
                  if decided { break; }
              }
              match ctx.state(|s| s.decision.clone()) {
                  Some(Decision::Approved) => { ctx.call(Reimburse, &expense).await?; Ok(Outcome::Approved) }
                  Some(Decision::Rejected(reason)) => { ctx.call(NotifyRejected, &reason).await?; Ok(Outcome::Rejected) }
                  None => { ctx.call(NotifyExpired, &expense).await?; Ok(Outcome::Expired) }
              }
          }

          #[signal]
          fn decide(&mut self, _ctx: &mut SyncWorkflowContext<Self>, input: (String, Decision)) {
              self.approver = Some(input.0);
              self.decision = Some(input.1);
          }

          #[query]
          fn status(&self, _ctx: &WorkflowContextView) -> (Option<Decision>, Option<String>) {
              (self.decision.clone(), self.approver.clone())
          }
      }
      """

  @implemented
  Scenario: Small expenses are approved without asking anyone
    When I execute "ExpenseApproval" for 5000 cents
    Then the workflow completes with "AutoApproved"
    And "RequestApproval" never ran

  @implemented
  Scenario: The manager approves within the deadline
    When I execute "ExpenseApproval" for 50000 cents
    And after 3 hours I signal "decide" with ("manager", Approved)
    Then the workflow completes with "Approved"
    And "Reimburse" ran once
    And "RequestApproval" ran once

  @implemented
  Scenario: No answer escalates up the chain
    When I execute "ExpenseApproval" for 50000 cents
    And 48 hours pass without a decision
    Then "RequestApproval" has run for "director"
    And querying "status" returns no decision and no approver
    When I signal "decide" with ("director", Rejected("no receipt"))
    Then the workflow completes with "Rejected"
    And "NotifyRejected" ran with "no receipt"

  @implemented
  Scenario: Nobody answers and the request expires
    When I execute "ExpenseApproval" for 50000 cents
    And 144 hours pass without a decision
    Then the workflow completes with "Expired"
    And "RequestApproval" ran 3 times

  @todo @crash
  Scenario: The escalation deadline survives a restart
    When I execute "ExpenseApproval" for 50000 cents
    And 40 hours pass
    And the engine is dropped and a new engine resumes the instance
    And 8 more hours pass
    Then "RequestApproval" has run for "director"

  @implemented @crash
  Scenario: A decision sent before a crash is not lost
    When I execute "ExpenseApproval" for 50000 cents
    And I signal "decide" with ("manager", Approved)
    And the engine is dropped before the workflow reacts
    And a new engine resumes the instance
    Then the workflow completes with "Approved"

  @todo
  Scenario: A second decision after the first is ignored and reported
    Given the manager approved and the workflow completed
    When I signal "decide" with ("cfo", Rejected("late"))
    Then the signal call fails because the instance is not running
