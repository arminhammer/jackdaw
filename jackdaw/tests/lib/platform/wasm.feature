@platform @wasm
Feature: Running on WebAssembly
  As a Rust developer
  I want to use jackdaw as a library in a program that compiles to WebAssembly
  So that workflows can run on WASI hosts such as wasmtime, Spin and wasmCloud

  # Each program is a small crate under tests/wasm, built for wasm32-wasip2 and run with wasmtime. These scenarios are
  # skipped when the target or wasmtime is missing.

  @implemented
  Scenario: A program that uses the native API builds for WASI
    Given the program "hello" of the fixture crate "core"
    When I build it for WASI
    Then the WASI build succeeds

  @implemented
  Scenario: A workflow runs to completion under wasmtime
    Given the program "hello" of the fixture crate "core"
    When I run it with wasmtime
    Then it prints "Hello, World!"
    And it exits successfully

  # WebAssembly has one thread: concurrent calls interleave there instead of running in parallel.
  @implemented
  Scenario: Concurrent calls, a bounded fan-out and a retry work on one thread
    Given the program "fan_out" of the fixture crate "core"
    When I run it with wasmtime
    Then it prints "price=30 stock=3 doubled=[2, 4, 6] flaky=42"
    And it exits successfully

  @implemented
  Scenario: A durable sleep waits for real time
    Given the program "sleeper" of the fixture crate "core"
    When I run it with wasmtime
    Then it prints "awake, after at least 1s: true"
    And it exits successfully

  # There are no threads to hand blocking work to, so a blocking task runs where it is called.
  @implemented
  Scenario: A blocking task runs, and limits still cap the calls that wait
    Given the program "blocking" of the fixture crate "core"
    When I run it with wasmtime
    Then it prints "checksum=10 rendered=4 most_at_once=1"
    And it exits successfully

  # WASI has no file locks, so nothing stops two engines opening one file there: use one engine per file.
  @implemented
  Scenario: A workflow resumes from a Redb file after the program stops during its timer
    Given the program "durable" of the fixture crate "core"
    When I run it with wasmtime, given a directory and the argument "start"
    Then it prints "stopped during the timer"
    And it exits successfully
    When I run it with wasmtime, given a directory and the argument "finish"
    Then it prints "shipped o-1 after charging 1 time(s)"
    And it exits successfully

  # The default features are the YAML interpreter, Redb and the HTTP client: the first two work on WASI, and the
  # HTTP client is left out there until WASI gets a client of its own.
  @implemented
  Scenario: A program that takes the default features builds for WASI
    Given the program "hello" of the fixture crate "defaults"
    When I build it for WASI
    Then the WASI build succeeds

  # The interpreter runs on WASI as it does natively: the same workflows give the same output. What needs a process or
  # an HTTP client is not available there (see below).
  @implemented
  Scenario: YAML workflows run on WASI and give the output they give natively
    Given the program "yaml" of the fixture crate "defaults"
    When I run it with wasmtime
    Then it prints the output that the native engine gives for "orders.sw.yaml"
    And that output has the prices 80 and 60
    And that output does not have the price 10
    And it prints the output that the native engine gives for "switch.sw.yaml"
    And that output has "size" "large"
    And it exits successfully

  # WebAssembly has no child processes and no HTTP client here, so what needs them fails, and says why.
  @implemented
  Scenario Outline: A YAML task that needs a process or an HTTP client fails and says it is not available
    Given the program "unsupported" of the fixture crate "defaults"
    When I run it with wasmtime
    Then the "<workflow>" workflow fails with "<message>"

    Examples:
      | workflow | message                                                 |
      | shell    | `run: shell` is not available on wasm                   |
      | http     | `call: http` is not available on wasm                   |
      | python   | `run: script` language `python` is not available on wasm |

  # These need an operating system (sockets, C libraries, Docker): the build stops and names the feature, instead of
  # failing inside a dependency.
  @implemented
  Scenario Outline: A feature that needs an operating system fails the WASI build with a message that names it
    Given the program "hello" of the fixture crate "defaults"
    When I build it for WASI with the feature "<feature>"
    Then the WASI build fails with "the `<feature>` feature is not available on wasm"

    Examples:
      | feature       |
      | sqlite        |
      | postgres      |
      | docker        |
      | http-listener |
      | grpc-listener |
      | openapi       |
