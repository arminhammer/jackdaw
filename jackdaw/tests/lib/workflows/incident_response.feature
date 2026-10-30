@workflows @incident
Feature: Incident response
  As a Rust developer building on-call tooling
  I want a workflow that pages responders, waits for acknowledgement, escalates and runs diagnostics in parallel
  So that every incident follows the same durable runbook

  Background:
    Given an engine with a Redb store and a controllable clock
    And the workflow:
      """rust
      #[workflow(namespace = "ops", name = "incident", version = "1.0.0")]
      struct Incident { acked_by: Option<String>, resolved: bool, notes: Vec<String> }

      #[workflow_methods]
      impl Incident {
          #[run]
          async fn run(ctx: &mut WorkflowContext<Self>, alert: Alert) -> WorkflowResult<Postmortem> {
              let opened = ctx.now();
              let (diag, owner) = futures::try_join!(
                  ctx.run(Diagnostics::run, &alert),          // child workflow
                  ctx.call(LookupOwner, &alert.service),
              )?;

              let mut level = 0;
              while ctx.state(|s| s.acked_by.is_none()) {
                  ctx.call(Page, &(owner.escalation[level].clone(), alert.clone())).await?;
                  jackdaw::select! {
                      _ = ctx.wait_condition(|s| s.acked_by.is_some()) => {},
                      _ = ctx.sleep(15.minutes()) => { level = (level + 1).min(owner.escalation.len() - 1); },
                  }
              }

              if alert.severity == Severity::Critical {
                  ctx.call(OpenStatusPage, &alert).await?;
              }

              let resolution = ctx.wait_for_event::<Resolution>("resolved").timeout(7.days()).await;
              let ended = ctx.now();
              let notes = ctx.state(|s| s.notes.clone());
              Ok(ctx.call(WritePostmortem, &(alert, diag, resolution.ok(), notes, ended - opened)).await?)
          }

          #[signal] fn ack(&mut self, _: &mut SyncWorkflowContext<Self>, who: String) { self.acked_by = Some(who); }
          #[signal] fn note(&mut self, _: &mut SyncWorkflowContext<Self>, text: String) { self.notes.push(text); }
          #[query]  fn acked_by(&self, _: &WorkflowContextView) -> Option<String> { self.acked_by.clone() }
      }
      """

  @implemented
  Scenario: The first responder acknowledges quickly
    When I execute "Incident" for a "Major" alert
    And after 5 minutes I signal "ack" with "sam"
    And I send the event "resolved" after 2 hours
    Then "Page" ran once
    And "OpenStatusPage" never ran
    And the postmortem has a duration of 2 hours

  @implemented
  Scenario: Diagnostics and owner lookup run in parallel
    When I execute "Incident"
    Then the "Diagnostics" child and "LookupOwner" overlapped in time

  @implemented
  Scenario: No acknowledgement escalates every 15 minutes
    When I execute "Incident" with an escalation chain of 3 people
    And 45 minutes pass without an "ack"
    Then "Page" ran 3 times, once per person in order
    When 15 more minutes pass
    Then the last person is paged again

  @implemented
  Scenario: Critical incidents open a status page
    When I execute "Incident" for a "Critical" alert and "ack" it
    Then "OpenStatusPage" ran once

  @implemented
  Scenario: Notes from responders end up in the postmortem
    When I execute "Incident", "ack" it and signal "note" with "rolled back deploy" and "cache flushed"
    And I send the event "resolved"
    Then the postmortem contains both notes in order

  @implemented
  Scenario: An unresolved incident still gets a postmortem after 7 days
    When I execute "Incident" and "ack" it
    And 7 days pass without "resolved"
    Then "WritePostmortem" ran with no resolution

  @todo @crash
  Scenario: Paging resumes on schedule after a restart
    When I execute "Incident" and 10 minutes pass without an "ack"
    And the engine is dropped and a new engine resumes the instance
    And 5 more minutes pass
    Then the second person has been paged
    And the first person was paged only once

  @implemented
  Scenario: The incident workflow passes the test engine's self-replay
    Given a "TestEngine" with self-replay enabled
    When I run the "first responder" and "escalation" scenarios on it
    Then both complete without "Error::NonDeterminism"
