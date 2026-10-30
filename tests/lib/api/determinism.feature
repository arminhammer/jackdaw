@api @determinism
Feature: Non-deterministic code is rejected in workflow bodies
  As a library user
  I want the compiler to stop me from breaking replay
  So that I find mistakes before production does

  @implemented
  Scenario Outline: A non-deterministic call in a run body is a compile error
    Given a "#[run]" body that calls "<call>"
    When I compile it
    Then it fails with a message that names "<call>" and suggests "<advice>"
    And the message says audited code can opt out with "#[allow_nondeterminism]"

    Examples:
      | call                       | advice                   |
      | std::time::SystemTime::now | ctx.now()                |
      | tokio::time::sleep         | ctx.sleep(duration)      |
      | tokio::spawn               | #[task]                  |
      | tokio::select              | jackdaw::select!         |
      | std::fs::read_to_string    | #[task]                  |
      | reqwest::get               | #[task]                  |

  @implemented
  Scenario: Non-determinism can be allowed explicitly
    Given a "#[run]" body that calls "SystemTime::now" in a statement marked "#[allow_nondeterminism]"
    When I compile it
    Then it compiles

  @implemented
  Scenario: Reordering steps is detected on replay
    Given a completed instance of a workflow that calls "A" then "B"
    When the code changes to call "B" then "A"
    And I call "Engine::replay_all"
    Then it returns one "Divergence" naming the instance and the first step that differs
    And the divergence names the source location of the step
