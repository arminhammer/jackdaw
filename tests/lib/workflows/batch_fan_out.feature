@workflows @batch
Feature: Batch import with fan-out and fan-in
  As a Rust developer building a data pipeline
  I want to split a file into chunks, process them in parallel and aggregate
  So that large imports are fast and partial failures are handled

  Background:
    Given an engine with a Redb store
    And the workflow:
      """rust
      #[workflow(namespace = "data", name = "import", version = "1.0.0")]
      struct Import;

      #[workflow_methods]
      impl Import {
          #[run]
          async fn run(ctx: &mut WorkflowContext<Self>, job: Job) -> WorkflowResult<Report> {
              let manifest = ctx.call(Split, &job).await?;                   // Vec<Chunk>
              let (mut ok, mut failed) = (Vec::new(), Vec::new());
              for wave in manifest.chunks.chunks(job.parallelism) {         // bounded fan-out
                  let results = futures::future::join_all(wave.iter().map(|c| ctx.call(ImportChunk, c))).await;
                  for (chunk, result) in wave.iter().zip(results) {
                      match result { Ok(r) => ok.push(r), Err(_) => failed.push(chunk.clone()) }
                  }
              }
              if failed.len() * 10 > manifest.chunks.len() {
                  ctx.call(Rollback, &job).await?;
                  return Err(TaskError::fault("validation").title("too many failed chunks").into());
              }
              for chunk in &failed {                                          // fan-in with retries
                  ok.push(ctx.call(ImportChunk, chunk).retry(Retry::exponential(3)).await?);
              }
              let report = ctx.call(Aggregate, &ok).await?;
              ctx.run(Notify::run, &report).await?;
              Ok(report)
          }
      }
      """

  @implemented
  Scenario: All chunks succeed
    When I execute "Import" with 40 chunks and parallelism 8
    Then the workflow completes with a report of 40 imported chunks
    And at most 8 "ImportChunk" calls ran at once
    And the "Notify" child workflow ran once

  @implemented
  Scenario: A few failed chunks are retried one by one
    Given chunks 3 and 17 fail on their first attempt only
    When I execute "Import" with 40 chunks
    Then the workflow completes with 40 imported chunks
    And "ImportChunk" ran 42 times

  @implemented
  Scenario: Too many failures roll back the import
    Given 6 of 40 chunks fail every time
    When I execute "Import"
    Then "Rollback" ran once
    And the workflow fails with "too many failed chunks"
    And "Aggregate" never ran

  @todo @crash
  Scenario: A crash mid-fan-out only re-imports unfinished chunks
    Given the engine is dropped after 25 of 40 chunks finished
    When a new engine resumes the instance
    Then the workflow completes
    And each chunk was imported exactly once in total

  @implemented
  Scenario: ctx.for_each is the shorthand when any failure should stop the batch
    Given a variant of "Import" that does "ctx.for_each(&manifest.chunks, ImportChunk).concurrency(8).await?"
    And chunk 5 fails non-retryably
    When I execute it with 40 chunks
    Then the workflow fails with chunk 5's fault
    And "Aggregate" never ran

  @todo @performance
  Scenario: A large fan-out stays fast
    When I execute "Import" with 5000 chunks and parallelism 64 on a Redb store
    Then it completes in under 30 seconds with no-op chunk tasks
    And the journal size grows linearly with the number of chunks
