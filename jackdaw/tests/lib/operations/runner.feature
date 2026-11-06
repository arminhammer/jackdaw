@operations @runner
Feature: Running a workflow as a program
  As the author of a pipeline built on jackdaw
  I want one call that starts, resumes or restarts a run and reports its progress
  So that my program needs no lifecycle code of its own, whether it runs in a terminal, in CI or as a service

  # A `Runner` wraps an engine. Its reporter decides what is shown: `Headless` shows nothing, `Lines` writes one line per
  # step, `Json` writes one JSON object per event, and `Terminal` (feature `terminal`) draws progress bars.

  Background:
    Given an engine with a Redb store

  Scenario: A headless run returns the output and shows nothing
    Given a runner with a headless reporter
    When the runner runs the "greeting" workflow as "run/a"
    Then the run's output is "hello, world!"
    And nothing was reported

  Scenario: A lines reporter shows each step as it finishes
    Given a runner with a lines reporter
    When the runner runs the "greeting" workflow as "run/a"
    Then the report says, in order:
      | starting run/a      |
      | runner.greet started |
      | runner.greet done |
      | runner.shout started |
      | runner.shout done |
      | run/a completed     |

  Scenario: A failing step is reported and the run's error is returned
    Given a runner with a lines reporter
    When the runner runs the "exploding" workflow as "run/a"
    Then the runner's run fails with "boom"
    And the report contains "runner.explode failed:"
    And the report contains "boom"
    And the report ends with "run/a failed"

  Scenario: A step that is tried again is reported with its attempt
    Given a runner with a lines reporter
    When the runner runs the "flaky" workflow as "run/a"
    Then the run's output is "steady"
    And the report contains "runner.flaky retrying after attempt 1"

  Scenario: A run that finished is not run again
    Given a runner with a lines reporter
    And the runner ran the "greeting" workflow as "run/a"
    When the runner runs the "greeting" workflow as "run/a"
    Then the run's output is "hello, world!"
    And the report contains "run/a has finished; its stored result follows"
    And the task "greet" ran 1 time

  Scenario: A run stopped by a crash is resumed
    Given a runner with a lines reporter
    And the runner started the "waiting" workflow as "run/a" and its engine then crashed
    When a new engine on the same store has a runner with a lines reporter
    And the runner runs the "waiting" workflow as "run/a" and sends it the event "go"
    Then the run's output is "released"
    And the report contains "resuming run/a"
    And the report contains "runner.prepare replayed"
    And the task "prepare" ran 1 time

  Scenario: Running again with different input is refused
    Given a runner with a lines reporter
    And the runner ran the "greeting" workflow as "run/a" with input "world"
    When the runner runs the "greeting" workflow as "run/a" with input "moon"
    Then the run is refused because its input changed
    And the refusal names the stored input "world" and the new input "moon"

  Scenario: A restart discards the earlier run
    Given a runner with a lines reporter
    And the runner ran the "greeting" workflow as "run/a" with input "world"
    When the runner restarts the "greeting" workflow as "run/a" with input "moon"
    Then the run's output is "hello, moon!"
    And the report contains "restarting run/a"
    And the task "greet" ran 2 times

  Scenario: A run that ended in failure is not silently run again
    Given a runner with a lines reporter
    And the runner ran the "exploding" workflow as "run/a"
    When the runner runs the "exploding" workflow as "run/a"
    Then the run is refused because it already ended as "Faulted"
    And the refusal suggests a restart

  Scenario: The status of a run is known without running it
    Given a runner with a lines reporter
    Then the status of "run/a" is none
    When the runner runs the "greeting" workflow as "run/a"
    Then the status of "run/a" is "Completed"

  Scenario: A step's line can be described by the program
    Given a runner with a lines reporter that describes "greet" output as "greeted <output>"
    When the runner runs the "greeting" workflow as "run/a"
    Then the report contains "greeted \"hello, world\""

  Scenario: A progress total gives each finished step its place
    Given a runner with a lines reporter that expects 2 steps
    When the runner runs the "greeting" workflow as "run/a"
    Then the report contains "runner.greet done [1/2]"
    And the report contains "runner.shout done [2/2]"

  Scenario: A JSON reporter writes one object per event
    Given a runner with a JSON reporter
    When the runner runs the "greeting" workflow as "run/a"
    Then every line of the report is a JSON object with an "event" field
    And the report's events are "WorkflowStarted", "TaskStarted", "TaskCompleted", "TaskStarted", "TaskCompleted", "WorkflowCompleted"

  Scenario: A run that takes too long ends with a timeout
    Given a runner with a lines reporter
    When the runner runs the "waiting" workflow as "run/a" with a timeout of 5 seconds
    Then the runner's run fails with "timed out"
    And the report ends with "run/a failed"

  Scenario: A lines reporter can show what each step returned
    Given a runner with a lines reporter that shows outputs
    When the runner runs the "greeting" workflow as "run/a"
    Then the report contains "runner.greet done"
    And the report contains "    \"hello, world\""

  @terminal
  Scenario: A terminal reporter shows how long each step took
    Given a runner with a terminal reporter drawing to a buffer
    When the runner runs the "greeting" workflow as "run/a"
    Then the drawn output mentions "runner.greet (0ms)"

  @terminal
  Scenario: A terminal reporter shows steps a resumed run did not redo
    Given a runner with a terminal reporter drawing to a buffer
    And the runner started the "waiting" workflow as "run/a" and its engine then crashed
    When a new engine on the same store has a runner with a terminal reporter drawing to a buffer
    And the runner runs the "waiting" workflow as "run/a" and sends it the event "go"
    Then the drawn output mentions "resuming run/a"
    And the drawn output mentions "runner.prepare (already completed)"

  @terminal
  Scenario: A terminal reporter can show what each step returned
    Given a runner with a terminal reporter that shows outputs
    When the runner runs the "greeting" workflow as "run/a"
    Then the drawn output mentions "hello, world"

  @terminal
  Scenario: Runs that share a display label their lines
    Given two runners whose terminal reporters share one display, labelled "first" and "second"
    When both runners run the "greeting" workflow as "run/a" and "run/b"
    Then the drawn output mentions "[first] ✔ runner.greet"
    And the drawn output mentions "[second] ✔ runner.greet"

  Scenario Outline: The reporter is chosen from where the program runs
    When the program runs with <where>
    Then the chosen reporter is "<reporter>"

    Examples:
      | where                                  | reporter |
      | a terminal                             | terminal |
      | its output piped to a file             | lines    |
      | a terminal and the quiet flag          | headless |
      | a terminal and the json flag           | json     |
      | a dumb terminal                        | lines    |

  @terminal
  Scenario: A terminal reporter draws progress and leaves a summary
    Given a runner with a terminal reporter drawing to a buffer
    When the runner runs the "greeting" workflow as "run/a"
    Then the run's output is "hello, world!"
    And the drawn output mentions "greet"
    And the drawn output ends with a summary of 2 steps done and none failed

  @cli
  Scenario: Command line flags choose the reporter and the restart
    When the arguments "--quiet --restart" are parsed
    Then the chosen reporter is "headless"
    And the restart flag is set
