@operations @config
Feature: A workflow's input from defaults, files, the environment and arguments
  As the author of a pipeline built on jackdaw
  I want the workflow's input type to be read from whichever sources the user chooses
  So that my program describes its inputs once and users pass them in the way that suits them

  # The input type is plain serde. `Layers` merges sources in the order they are added: later ones win. The usual order
  # is defaults (serde's), files, the environment, then arguments.

  Scenario: Serde defaults fill what no source gives
    Given a settings file "base.toml" containing:
      """
      source = "s3://bucket"
      """
    When the settings are loaded from "base.toml"
    Then the settings are source "s3://bucket", workers 4, verbose false

  Scenario Outline: A file in any supported format
    Given a settings file "<file>" containing:
      """
      <content>
      """
    When the settings are loaded from "<file>"
    Then the settings are source "from-file", workers 9, verbose true

    Examples:
      | file       | content                                                  |
      | s.toml     | source = "from-file"\nworkers = 9\nverbose = true        |
      | s.json     | {"source": "from-file", "workers": 9, "verbose": true}   |
      | s.yaml     | source: from-file\nworkers: 9\nverbose: true             |
      | s.yml      | source: from-file\nworkers: 9\nverbose: true             |

  Scenario: Later files override earlier ones
    Given a settings file "a.toml" containing:
      """
      source = "a"
      workers = 2
      """
    And a settings file "b.toml" containing:
      """
      workers = 7
      """
    When the settings are loaded from "a.toml" then "b.toml"
    Then the settings are source "a", workers 7, verbose false

  Scenario: The environment overrides a file
    Given a settings file "base.toml" containing:
      """
      source = "file"
      workers = 2
      """
    And the environment has:
      | name           | value |
      | JD_WORKERS     | 8     |
      | JD_VERBOSE     | true  |
      | OTHER_WORKERS  | 99    |
    When the settings are loaded from "base.toml" and the "JD_" environment
    Then the settings are source "file", workers 8, verbose true

  Scenario: Arguments override the environment, and unset arguments override nothing
    Given a settings file "base.toml" containing:
      """
      source = "file"
      workers = 2
      """
    And the environment has:
      | name       | value |
      | JD_WORKERS | 8     |
    And the arguments are:
      | name    | value |
      | source  | arg   |
    When the settings are loaded from "base.toml", the "JD_" environment and the arguments
    Then the settings are source "arg", workers 8, verbose false

  Scenario: Nested settings take a double underscore in the environment
    Given the environment has:
      | name              | value  |
      | JD_SOURCE         | x      |
      | JD_LIMITS__RETRIES | 5     |
    When the settings are loaded from the "JD_" environment
    Then the settings are source "x", workers 4, verbose false
    And the retries limit is 5

  Scenario: A missing required setting is named
    When the settings are loaded from no source
    Then loading fails and the message mentions "source"

  Scenario: A value of the wrong type is named
    Given a settings file "bad.toml" containing:
      """
      source = "x"
      workers = "many"
      """
    When the settings are loaded from "bad.toml"
    Then loading fails and the message mentions "workers"

  Scenario: A file that does not exist is an error
    When the settings are loaded from "nope.toml"
    Then loading fails and the message mentions "nope.toml"

  Scenario: An optional file that does not exist is skipped
    Given the environment has:
      | name      | value |
      | JD_SOURCE | x     |
    When the settings are loaded from the optional "nope.toml" and the "JD_" environment
    Then the settings are source "x", workers 4, verbose false

  Scenario: A file with an unknown extension is an error
    Given a settings file "s.ini" containing:
      """
      source = x
      """
    When the settings are loaded from "s.ini"
    Then loading fails and the message mentions ".ini"

  # `#[derive(Config)]` gives the input type its flags: one per field, named after it, with the doc comment as help.
  # `Program` (feature `cli`) reads the input from `--config` files, the environment (`BACKUP_` for a workflow named
  # "backup") and those flags, in that order, then runs the workflow with a `Runner`.

  Scenario: Flags give the input
    Given a backup program
    When the backup program is run with "--quiet --source s3://b --workers 8"
    Then the program prints "s3://b:8:false::0"

  Scenario: A short flag, a list and a switch
    Given a backup program
    When the backup program is run with "--quiet --source x -w 2 --tags a --tags b --verbose"
    Then the program prints "x:2:true:a,b:0"

  Scenario: A switch can be turned off by a flag
    Given a settings file "on.toml" containing:
      """
      source = "x"
      verbose = true
      """
    And a backup program
    When the backup program is run with "--quiet --config {dir}/on.toml --verbose=false"
    Then the program prints "x:4:false::0"

  Scenario: Files, the environment and flags are layered
    Given a settings file "base.toml" containing:
      """
      source = "file"
      workers = 2
      tags = ["f"]
      """
    And the environment has:
      | name           | value |
      | BACKUP_WORKERS | 8     |
      | BACKUP_SOURCE  | env   |
    And a backup program
    When the backup program is run with "--quiet --config {dir}/base.toml --source flag"
    Then the program prints "flag:8:false:f:0"

  Scenario: A field marked skip has no flag but still reads from files and the environment
    Given the environment has:
      | name                    | value |
      | BACKUP_LIMITS__RETRIES  | 5     |
    And a backup program
    When the backup program is run with "--quiet --source x"
    Then the program prints "x:4:false::5"
    When the backup program is run with "--source x --limits.retries 5"
    Then the program fails with usage error mentioning "--limits.retries"

  Scenario: Help lists the flags with the fields' doc comments
    Given a backup program
    When the backup program is run with "--help"
    Then the program shows usage mentioning:
      | --source  |
      | Where to read from |
      | --workers |
      | --config  |
      | --restart |
    And the usage does not mention "--limits"

  Scenario: A missing required setting is reported
    Given a backup program
    When the backup program is run with "--quiet"
    Then the program fails with a configuration error mentioning "source"

  Scenario: A flag of the wrong type is a usage error
    Given a backup program
    When the backup program is run with "--quiet --source x --workers many"
    Then the program fails with usage error mentioning "many"

  Scenario: The same id with the same input returns the stored result
    Given a backup program
    When the backup program is run with "--quiet --run-id nightly --source x"
    And the backup program is run with "--quiet --run-id nightly --source x"
    Then the program prints "x:4:false::0"

  Scenario: The same id with other input is refused unless restarted
    Given a backup program
    When the backup program is run with "--quiet --run-id nightly --source x"
    And the backup program is run with "--quiet --run-id nightly --source y"
    Then the program fails with a run error mentioning "input"
    When the backup program is run with "--quiet --run-id nightly --source y --restart"
    Then the program prints "y:4:false::0"
