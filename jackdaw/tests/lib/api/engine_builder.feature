@api @engine
Feature: Building an engine
  As a library user
  I want to configure storage and registrations in one place
  So that mistakes are reported before any workflow runs

  @implemented
  Scenario: An engine without a store keeps everything in memory
    Given an engine built with "Engine::builder().build()"
    When I execute a workflow to completion
    And I drop the engine and build a new one the same way
    Then the new engine reports the instance as unknown

  @implemented
  Scenario: An engine with a Redb store keeps instances across engines
    Given an engine built with a "Redb" store at a temporary path
    When I execute a workflow to completion
    And I drop the engine and build a new one on the same path
    Then "Engine::status" of the instance is "Completed"

  @implemented
  Scenario Outline: Typed stores exist for every persistence backend
    Given an engine built with "<store>"
    When I execute a workflow to completion
    And I drop the engine and build a new one on the same store
    Then "Engine::status" of the instance is "Completed"
    And a task result cached by the first engine is served from the cache by the second

    Examples:
      | store                                   |
      | Redb::open(path)                        |
      | Sqlite::open(path)                      |

  # Needs a Postgres server: run with DATABASE_URL set and `--tags @postgres`.
  @todo @postgres
  Scenario Outline: The Postgres store keeps instances across engines
    Given an engine built with "<store>"
    When I execute a workflow to completion
    And I drop the engine and build a new one on the same store
    Then "Engine::status" of the instance is "Completed"
    And a task result cached by the first engine is served from the cache by the second

    Examples:
      | store                                   |
      | Postgres::connect(url)                  |

  @implemented
  Scenario: Third-party stores implement a Store trait
    Given a type that implements "jackdaw::api::Store" using in-memory maps
    When I pass it to "EngineBuilder::store"
    Then the engine persists journals and cached results through it

  @implemented
  Scenario: The builder accepts raw providers
    Given a "SqlitePersistence" and a "SqliteCache" from "jackdaw::providers"
    When I pass them to "EngineBuilder::persistence" and "EngineBuilder::cache"
    Then the engine builds

  @implemented
  Scenario: The builder rejects a workflow that calls an unregistered task
    Given a workflow whose "#[run]" calls the task "Charge"
    When I build an engine that registers the workflow but not "Charge"
    Then "build" fails with "Error::Invalid"
    And the message names "Charge"

  @implemented
  Scenario: The builder registers a child workflow with its parent
    Given a workflow whose "#[run]" runs the child workflow "Payment"
    When I build an engine that registers only the parent
    Then the engine builds and can run the parent with its child

  @implemented
  Scenario: The builder rejects a catalog entry that does not exist
    Given a "#[catalog]" that imports a task the catalog does not contain
    When I build an engine with that catalog
    Then "build" fails with "Error::Invalid"

  @implemented
  Scenario: A slow event subscriber never blocks a run
    Given an engine built with "event_buffer_size(1)"
    When a workflow emits 10 events and nobody reads them
    Then the workflow completes
    And a subscriber that joins after completion receives all 10 events in order
