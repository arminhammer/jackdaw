# Changelog

All notable changes to jackdaw are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/) (before 1.0, a minor version may break the API; see
CONTRIBUTING.md for what is considered stable).

The workspace publishes three crates with the same version: `jackdaw-wf` (the library, imported as `jackdaw`),
`jackdaw-wf-macros` and `jackdaw-cli` (the `jackdaw` binary).

## [Unreleased]

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

[Unreleased]: https://github.com/arminhammer/jackdaw/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/arminhammer/jackdaw/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/arminhammer/jackdaw/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/arminhammer/jackdaw/releases/tag/v0.1.1
