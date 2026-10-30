@interop @extensions
Feature: Custom executors and persistence providers
  As a library user
  I want to plug in my own call types and storage
  So that jackdaw fits my infrastructure

  @todo
  Scenario: A custom call type is registered on the builder
    Given an executor "kafka" that implements "Executor" and records each call
    When I build an engine with "EngineBuilder::executor(\"kafka\", executor)"
    And a YAML workflow with "call: kafka" and arguments "{ topic: orders }"
    Then the executor receives the task name and the arguments
    And its return value is the task's output

  @todo
  Scenario: Registering two executors with the same name fails
    When I register "kafka" twice
    Then "build" fails with "Error::Invalid" naming "kafka"

  @todo
  Scenario: The Executor trait has no downcasting or console plumbing
    When I implement "jackdaw::Executor" for a new type
    Then I implement only "exec" and optionally "cacheable"
    And "exec" has no "TaskOutputStreamer" parameter
    And the trait has no "as_any" method

  @todo
  Scenario: A custom executor's results can be cached
    Given a custom executor whose "cacheable" returns true
    When two instances call it with the same arguments
    Then it is invoked once

  @implemented
  Scenario: A custom persistence provider is used for journals
    Given a type that implements "PersistenceProvider" with in-memory maps
    When I build an engine with "EngineBuilder::persistence(Arc::new(provider))"
    And a workflow runs to completion
    Then the provider holds the instance's checkpoint and events

  @implemented
  Scenario: A custom cache provider is used for cached tasks
    Given a type that implements "CacheProvider" that counts lookups
    When I build an engine with "EngineBuilder::cache(Arc::new(provider))"
    And a cached task runs twice with equal input
    Then the provider served the second result

  @todo
  Scenario: The documented custom PersistenceProvider example compiles and runs
    When I run "cargo run -p jackdaw --example custom_persistence"
    Then it completes successfully

  @todo
  Scenario: The documented custom Executor example compiles and runs
    When I run "cargo run -p jackdaw --example custom_executor"
    Then it completes successfully
