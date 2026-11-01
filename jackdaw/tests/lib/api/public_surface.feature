@api @surface
Feature: The public API is small, documented and stable
  As a library user
  I want one obvious way in
  So that upgrades do not break me because of internals I happened to use

  @implemented
  Scenario: One import brings in what an application needs
    Given a crate that depends on "jackdaw"
    When it contains only "use jackdaw::prelude::*;"
    Then it can name each of:
      | item                |
      | Engine              |
      | EngineBuilder       |
      | ExecutionHandle     |
      | WorkflowContext     |
      | TaskContext         |
      | TaskError           |
      | WorkflowError       |
      | WorkflowResult      |
      | Error               |
      | Retry               |
      | DurationExt         |
      | Event               |
      | Status              |
      | Redb                |
      | task                |
      | tasks               |
      | workflow            |
      | workflow_methods    |
      | catalog             |

  @implemented
  Scenario: The crate root re-exports the curated set
    Given a crate that depends on "jackdaw"
    Then it can name each of these from the crate root:
      | item                |
      | Engine              |
      | EngineBuilder       |
      | ExecutionHandle     |
      | PersistenceProvider |
      | CacheProvider       |
      | Executor            |
      | Error               |

  @implemented
  Scenario Outline: Engine internals are not public
    Given a crate that depends on "jackdaw" with all features
    When it refers to "<path>"
    Then it fails to compile because the item is private

    Examples:
      | path                                  |
      | jackdaw::durableengine::DurableEngine |
      | jackdaw::expressions                  |
      | jackdaw::task_ext                     |
      | jackdaw::descriptors                  |
      | jackdaw::executionhistory             |
      | jackdaw::context::Context             |

  @implemented
  Scenario: Macro plumbing is hidden from the documented surface
    When I build the docs for "jackdaw"
    Then the items used only by generated macro code are under a "#[doc(hidden)]" module named "__private"
    And the "api" module page does not list any of:
      | item               |
      | Registry           |
      | register_workflow  |
      | CoreRef            |
      | NativeTask         |
      | RegisterTask       |
      | TaskRef            |
      | AsDependency       |
    And the macros still expand and compile against "::jackdaw::__private"

  @implemented
  Scenario: Every public item is documented
    When I run "cargo doc -p jackdaw-wf --all-features --no-deps" with "RUSTDOCFLAGS=-D warnings"
    Then the command succeeds
    And "src/lib.rs" contains "#![warn(missing_docs)]"

  @implemented
  Scenario Outline: Public enums can grow without a breaking change
    Then "<type>" is marked "#[non_exhaustive]"
    And a downstream "match" on "<type>" without a wildcard arm fails to compile

    Examples:
      | type                 |
      | jackdaw::api::Error  |
      | jackdaw::api::Event  |
      | jackdaw::api::Status |

  @implemented
  Scenario: Error enums of the extension traits can grow without a breaking change
    Then "jackdaw::persistence::Error" is marked "#[non_exhaustive]"
    And "jackdaw::cache::Error" is marked "#[non_exhaustive]"

  @todo
  Scenario: The public API is checked for semver breaks
    Given a published version of "jackdaw"
    When I run "cargo semver-checks -p jackdaw-wf" against it
    Then it reports no breaking change unless the minor version was bumped
