@steps @conditionals
Feature: Conditional branching in Rust workflows
  As a Rust developer
  I want to branch with if and match on step results
  So that control flow is ordinary Rust and still durable

  Background:
    Given an engine with a Redb store
    And a workflow "Router" whose run does:
      """rust
      let risk = ctx.call(Score, &order).await?;
      let decision = match risk {
          Risk::Low => ctx.call(AutoApprove, &order).await?,
          Risk::Medium if order.cents < 10_000 => ctx.call(AutoApprove, &order).await?,
          Risk::Medium | Risk::High => ctx.call(ManualReview, &order).await?,
      };
      if decision.approved {
          ctx.call(Ship, &order).await?;
      }
      Ok(decision)
      """

  @implemented
  Scenario Outline: The branch follows the step result
    Given "Score" returns "<risk>" for an order of <cents> cents
    And "ManualReview" approves
    When I execute "Router"
    Then the tasks that ran are <tasks>

    Examples:
      | risk   | cents | tasks                             |
      | Low    | 500   | score, auto_approve, ship         |
      | Medium | 500   | score, auto_approve, ship         |
      | Medium | 50000 | score, manual_review, ship        |
      | High   | 500   | score, manual_review, ship        |

  @implemented
  Scenario: A false condition skips the step entirely
    Given "Score" returns "High" and "ManualReview" rejects
    When I execute "Router"
    Then "ship" never ran
    And the journal has no step for "ship"

  @implemented
  Scenario: Replay takes the branch that was recorded
    Given a completed "Router" instance where "Score" returned "High"
    When "Score" would now return "Low"
    And I replay the instance
    Then the replay takes the "ManualReview" branch
    And "score" is not called during the replay

  @implemented @crash
  Scenario: A resumed instance continues inside the branch it was in
    Given "Score" returned "High" and the engine is dropped while "ManualReview" runs
    When a new engine resumes the instance
    Then "ManualReview" runs to completion
    And "AutoApprove" never runs

  @implemented
  Scenario: Branching on input alone needs no step
    Given a workflow that does "if input.express { ctx.call(Courier, &input).await? } else { ctx.call(Post, &input).await? }"
    When I execute it with "express: true"
    Then only "courier" ran

  @implemented
  Scenario: Changing a branch condition in new code is detected
    Given completed instances that took the "AutoApprove" branch for "Medium" risk
    When the code changes the guard to "order.cents < 100"
    And I run "Engine::replay_all"
    Then each affected instance is reported as a divergence at the branch's first step

  @implemented
  Scenario: patched lets a new branch apply only to new instances
    Given a running instance started before the code added "if ctx.patched(\"fraud-check\") { ctx.call(FraudCheck, &order).await?; }"
    When it resumes on the new code
    Then "fraud_check" does not run for it
    And a new instance runs "fraud_check"
