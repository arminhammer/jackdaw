# Changelog

All notable changes to jackdaw are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/) (before 1.0, a minor version may break the API; see
CONTRIBUTING.md for what is considered stable).

The workspace publishes three crates with the same version: `jackdaw-wf` (the library, imported as `jackdaw`),
`jackdaw-wf-macros` and `jackdaw-cli` (the `jackdaw` binary).

## [Unreleased]

## [0.2.4] - 2026-11-06

### Added

- A workflow's input is its configuration. `#[derive(Config)]` (feature `config`) gives the input type of a workflow
  one command line flag per field, named after it, with the field's doc comment as help. `Option<T>`, `Vec<T>` and
  `bool` (a switch) are understood, `#[config(skip)]` leaves a field without a flag (nested structs still come from
  files and the environment), and `#[config(short = 'w')]` and `#[config(long = "..")]` name a flag.
- `jackdaw::config::Layers`: reads any serde type from defaults, files (`.toml`, `.json`, `.yaml`, `.yml`, by
  extension), environment variables (`PREFIX_NAME`, `__` for nested settings) and arguments, each overriding the ones
  before. Errors name the setting or the file.
- `jackdaw::runner::Program` (feature `cli`): `main` for a workflow. It reads the input from `--config` files, the
  environment (`<WORKFLOW>_` by default) and the derived flags, runs the workflow with a `Runner` and prints the output
  as JSON. It has `--config`, `--run-id`, `--quiet`, `--json`, `--plain` and `--restart` of its own. `Program::run`
  does the same for arguments and variables you give it, for tests.
- Features `config` (figment) and `cli` now includes it. A guide chapter, `guide::config`, covers the pattern.

### Changed

- The README, the crate documentation and all examples take their input from a `Config` struct.

## [0.2.3] - 2026-11-05

### Changed

- `jackdaw run` shows its progress through `jackdaw::runner`, from the run's events: a progress bar and a line per step
  on standard error on a terminal, plain lines when piped, and the result on standard output. New flags: `--quiet`,
  `--json` (one JSON object per event), `--plain`, `--id` and `--restart` (a run with an id continues, or returns its
  result if it finished). `--debug` shows what each step returned in addition to each task's input and cache use.
  Failures are reported on standard error, not standard output.

### Added

- `jackdaw::runner`: a `Runner` that runs a workflow as a program. `Runner::run` starts the run, continues one a crash
  stopped (reporting the steps already done as replayed), returns the stored output of one that finished, and refuses one
  that was started with other input or ended in failure unless asked to restart; `Runner::status` tells where a run is.
  What it shows is up to a `Reporter`: `Headless`, `Lines` (one line per step, with `.total(n)` and `.describe(..)`),
  `Json` (one object per event) and, with the new `terminal` feature, `Terminal` (a progress bar from `indicatif`).
  `Env::detect().kind()` picks one from where the program runs, and the new `cli` feature adds `RunnerArgs`
  (`--quiet`, `--json`, `--plain`, `--restart`) for `clap` programs. The default build gains no dependency. See the guide
  chapter `guide::runner`.
- `RunOptions::timeout`; `Lines::outputs` and `Terminal::outputs` (show what each step returned); `Terminal` shows how long
  each step took and which steps a resumed run did not redo, and `Terminal::in_group` / `label` / `group` let several runs
  share one display.
- A YAML run's events include its tasks (`TaskStarted`, `TaskCompleted`, `TaskFaulted`, `TaskRetried`) as they happen,
  not only its start and end; steps done before a resume are reported as replayed.
- `Engine::execute_with` with the id of a YAML instance the store already has continues it if it was running, or returns
  its stored outcome if it had ended, as for a Rust workflow (it used to fail with "already exists"). `Engine::purge`
  works for a finished YAML instance.
- `InstanceDetails::input`: the input a Rust workflow was started with.
- WebAssembly: the library builds for `wasm32-wasip2` (wasmtime, Spin, wasmCloud) and runs there. The native API, the YAML
  interpreter (`set`, `for`, `fork`, `switch`, `call:` of Rust tasks, ...) and the Redb store work, in a program that uses
  tokio's `current_thread` runtime. The new guide chapter `guide::wasm` says what works and what does not.
  - What needs a child process or an HTTP client is left out on wasm: `ctx.run_shell`, `ctx.run_script`,
    `ctx.run_container` and `ctx.http` (with `Shell`, `Script`, `ProcessOutput`, `Request` and `Response`) are not
    compiled, and YAML `run: shell`, `run: script`, `call: http` and remote catalogs fail with a message that says they
    are not available on wasm. The `http-client` feature does nothing there.
  - Building with `sqlite`, `postgres`, `docker`, `http-listener`, `grpc-listener` or `openapi` for wasm stops with a
    message that names the feature.
  - `#[task(blocking)]` tasks and Redb's reads and writes run where they are called, as wasm has no threads to hand them
    to.
- `just wasm` and a `WebAssembly (WASI)` CI job check the library for `wasm32-wasip2` and run example programs under
  wasmtime.

### Changed

- The dependencies that need an operating system (`sqlx`, `axum`, `tonic` and the gRPC crates, `bollard`,
  `openapiv3-extended`, `jsonschema`, `reqwest`) are declared for non-wasm targets only. Nothing changes for native
  builds, except that tokio's `rt-multi-thread` feature is now enabled on native targets whatever the features (it adds no
  crates), and the `yaml` feature no longer asks for it. tokio's `fs` feature is no longer enabled: files are read with
  `std::fs` on the blocking pool.

## [0.2.2] - 2026-11-02

### Added

- Named limits on how many task calls run at once. `#[task(limit = "name")]` (async or blocking, free functions and
  `#[tasks]` methods) names a limit, and `EngineBuilder::limit(name, permits)` defines it for every instance the
  engine runs. `EngineBuilder::blocking_limit(permits)` limits the blocking tasks that name none. Calls wait for a
  permit in order without holding a thread. A YAML workflow's `call:` of a Rust task waits for the task's limit too.
  `build` fails when a task names a limit the engine does not define.
- With the `metrics` feature, the `jackdaw_task_limit_waiting` gauge counts the calls waiting for each limit.
- An engine with leases logs a warning when its lease renewal runs late because the async runtime's threads were
  busy (for more than a quarter of the lease TTL). The warning points at code that blocks inside an async task, the
  usual cause, before another engine takes the instances over.

### Changed

- Each call of an async task runs as a tokio task of its own. The calls a workflow makes at once
  (`futures::try_join!`, `join_all`, `ctx.for_each(..).concurrency(n)`) used to share the workflow's tokio task, and
  so one thread; they now run in parallel. Dropping a call's future still stops it.

## [0.2.1] - 2026-11-01

### Fixed

- The YAML interpreter's HTTP listener passes path parameters under the names the route gives them, and adds the
  query string, for every method. It used to report the last path segment of a GET or DELETE as `petId`, whatever
  the route said, and drop a GET or DELETE body. Engine listeners and YAML listeners now build a request's input the
  same way.
- `#[catalog]` in a build without the `yaml` feature fails with one error that names the feature, instead of errors
  about missing `__private` items.
- The docs build without warnings when some features are off.
- YAML instances appear in `Engine::list`, `Engine::status` and `Engine::describe`, read from the YAML engine's
  stored events. Before, those saw only Rust instances.
- With a `PayloadCodec` (`EngineBuilder::codec`), the YAML interpreter's records go through it as well: event
  payloads (inputs, task results, outputs, published events) and checkpoint data. Before, only Rust instances'
  journals and history were encoded, and a YAML workflow's data was stored as plain JSON. Records written before
  still read.
- With leases on, the YAML interpreter's own store writes for an instance are checked against the lease, as a
  Rust instance's are. An engine that lost the lease has its writes refused. Before, a stalled engine could write
  until its next renewal noticed.
- `Engine::sweep` also purges finished YAML instances older than the retention period. It also deletes the events
  published on the engine, which are now kept in one store stream per day, after the same period.
- The YAML interpreter stamps every event and checkpoint it stores with the engine's clock. Some used to take the
  wall clock, so a custom or test clock gave an instance timestamps from two clocks.
- The YAML interpreter's own HTTP listener checks requests against the listen task's OpenAPI document, as engine
  listeners do. The operation is the document path that the listen URI's path ends with (`/api/v1/add` matches
  `/add`); without one, the body schema named by `resource.name` applies. A document that cannot be read is
  skipped with a warning.
- YAML instances are leased like Rust ones under `EngineBuilder::lease`. An engine takes the lease before it starts
  or resumes one, and renews it while the run lasts. `Engine::recover` skips an instance whose lease another engine
  holds. Before, two engines could both resume the same YAML instance. The YAML interpreter writes to the store
  directly, so its writes are not checked against the lease. An engine that loses a lease stops the run at its next
  renewal.
- YAML runs, the readers of a script's output, and listener servers keep the caller's `tracing` subscriber, as Rust
  workflows already did.
- A YAML `listen` task for events emitted on the engine follows more of the spec:
  - A string in `with` is a regular expression that the whole attribute must match.
  - A runtime expression in `with` is evaluated against the workflow's data, and its value is matched the same way.
  - `correlate` is supported. A `from` value must equal `expect`, or without `expect`, the value the first matching
    event gave.
  - A listen task that resumes after a restart also sees the events emitted since it first started. The engine keeps
    published events in the store for this.
  - A YAML `emit` task stamps `time` from the engine's clock.
  - Before, `with` matched only by equality, `correlate` was ignored, and an event emitted while the listening
    instance's engine was down was lost.

### Changed

- The library no longer prints. The YAML interpreter's trace, the lines a script task prints and listener startup
  are `tracing` events on the `jackdaw::yaml` target (see "Logs and traces" in the events chapter of the guide).
  Before, a YAML script task's output went to the process's stdout and stderr, whatever the application did.
  `set_yaml_trace` is deprecated and does nothing. The gRPC listener's request diagnostics and the OpenAPI caller's
  request lines are `DEBUG` logs.
- Engine OpenAPI listeners (`Listener::openapi`) check each request against its operation before a workflow
  starts:
  - required parameters;
  - the schemas of path, query and header parameters;
  - the JSON request body's schema, with `$ref`s into `components` resolved. An `openapi: 3.0` document's schemas are
    read as JSON Schema draft 4, a 3.1 one's as 2020-12.
  A request that does not match gets a `400` listing the problems. A document with an invalid schema is rejected
  when the listener is built. This adds the `jsonschema` dependency to the `http-listener` feature, without its
  default features (no remote `$ref`s, no C code). The listener docs now say to terminate TLS in a proxy or
  sidecar.
- The `jackdaw` CLI renders that trace as before, and now also prints warnings and errors from the library on stderr
  (everything with `--verbose`; `RUST_LOG` overrides).
- `console` is no longer a dependency of the library.
- CI checks the public API against the latest release with `cargo semver-checks`, and dependency licenses and
  sources with `cargo deny` (`deny.toml`).

## [0.2.0] - 2026-11-01

Jackdaw becomes a library first: durable workflows written in Rust, with the Open Workflow YAML interpreter as
an optional part of the same engine. The CLI is now a separate crate built on the library's public API.

### Added

- The `jackdaw::api` and `jackdaw::prelude` API: `Engine` and `EngineBuilder`; typed tasks and workflows with
  `#[task]`, `#[tasks]`, `#[workflow]` and `#[workflow_methods]`; `WorkflowContext` (`call`, `run`, `start`,
  `for_each` with a concurrency limit, `race`, `sleep`, `wait_for_event`, `wait_condition`, `continue_as_new`,
  `patched`, `now`, `random`, `uuid`, `emit`); signals, queries and updates; retries, timeouts, heartbeats and caching
  per step; `ExecutionHandle`; and the `ctx.http`, `ctx.run_shell`, `ctx.run_script` and `ctx.run_container` helpers.
- Durability: a journal per instance (a header plus an append-only log), replay with compile-time checks against
  non-deterministic calls, `Engine::resume`, `Engine::recover`, `Engine::replay_all`, durable timers, persisted
  events, idempotent starts (`execute_with_id`, `ExecuteOptions`), workflow deadlines, graceful `shutdown`,
  retention, and leases so several engines can share one store.
- Stores: `Redb`, `Sqlite` and `Postgres` behind the `Store` trait, and `PersistenceProvider`/`CacheProvider` for your
  own; `PayloadCodec` to encode what is stored.
- Schedules (`Schedule::cron`, `Schedule::every`, with catch-up and overlap policies), including YAML workflows'
  `schedule` blocks (`Schedule::from_workflow`).
- Catalogs with typed bindings (`#[catalog]`).
- Custom call types for YAML (`EngineBuilder::executor` and the `Executor` trait).
- `api::Listener`: the engine serves an OpenAPI document's operations or a gRPC service, each request starting a
  workflow (`EngineBuilder::listener`, `Engine::listener_addr`).
- YAML workflows on the same engine: `Workflow::from_yaml` and `Workflow::validate`, `ctx.run_definition`, YAML
  workflows that call Rust tasks, recovery of YAML instances after a restart, and `listen` tasks that receive events
  emitted on the engine (by `ctx.emit` or YAML `emit`).
- `jackdaw::testing::TestEngine`: a virtual clock, self-replay checks, simulated crashes, and fake tasks.
- Observability: `tracing` spans per instance and step, and `metrics` counters and histograms (`metrics` feature).
- Cargo features: `yaml`, `redb` and `http-client` by default; `sqlite`, `postgres`, `docker`, `openapi`,
  `http-listener`, `grpc-listener` and `metrics` on request; `full` for everything.
- `jackdaw::dsl`, the Open Workflow DSL 1.0.3 types, vendored from the Rust SDK (`just update-dsl` refreshes it).
- A guide in the rustdoc (`jackdaw::guide`) and runnable examples.
- `jackdaw visualize` now renders a workflow, and with `-i` the task states of a stored run.

### Changed

- **Breaking:** the project is a workspace of three crates. The library is published as `jackdaw-wf` (the name
  `jackdaw` on crates.io belongs to another project) and is still imported as `jackdaw`.
- **Breaking:** the YAML interpreter's types (`DurableEngine`, `DurableEngineBuilder`, the expression and context
  modules) are private; YAML workflows run through `Engine`.
- **Breaking:** the standard error types are the URIs the specification's DSL reference gives,
  `https://open-workflow-specification.org/spec/1.0.0/errors/<type>` (`api::error_types`); 0.1 raised
  `https://serverlessworkflow.io/dsl/errors/types/<type>`. A YAML `catch` still matches the older spellings.
- The YAML interpreter uses the engine's clock, saves a checkpoint when an instance starts, stores failures, and stops
  its runs when the engine is dropped.
- YAML HTTP and gRPC listeners fail the workflow when they cannot bind their port (before, the error was only logged),
  and the HTTP listener uses axum 0.8.
- `jackdaw run` prints its result when the output is not a terminal.
- The minimum supported Rust version is 1.94.
- **Breaking:** Redb stores use redb 4, whose file format replaces the one redb 2 wrote; a Redb database from 0.1
  cannot be opened.
- YAML is parsed with `serde-saphyr`, a pure-Rust parser, instead of the deprecated `serde_yaml` (and the
  unmaintained `unsafe-libyaml` under it).
- The documentation uses the specification's new name, the Open Workflow Specification (formerly Serverless Workflow),
  and links to open-workflow-specification.org and the `open-workflow-specification` GitHub organization.

### Removed

- The Python bindings.
- The `.ci` pipeline written as a jackdaw workflow, and its `just ci`, `ci-clean` and `ci-reset` recipes (CI runs on
  GitHub Actions).
- `serverless_workflow_core` as a git dependency (its models are vendored as `jackdaw::dsl`).

## [0.1.1] - 2025-12-31

The CLI release before the library refactor; see the GitHub release.

[Unreleased]: https://github.com/arminhammer/jackdaw/compare/v0.2.4...HEAD
[0.2.4]: https://github.com/arminhammer/jackdaw/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/arminhammer/jackdaw/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/arminhammer/jackdaw/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/arminhammer/jackdaw/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/arminhammer/jackdaw/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/arminhammer/jackdaw/releases/tag/v0.1.1
