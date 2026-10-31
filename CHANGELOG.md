# Changelog

All notable changes to jackdaw are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/) (before 1.0, a minor version may break the API; see
CONTRIBUTING.md for what is considered stable).

The workspace publishes three crates with the same version: `jackdaw-wf` (the library, imported as `jackdaw`),
`jackdaw-wf-macros` and `jackdaw-cli` (the `jackdaw` binary).

## [Unreleased]

## [0.2.0] - unreleased

Set the date when the release is tagged.

Jackdaw becomes a library first: durable workflows written in Rust, with the Serverless Workflow YAML interpreter as
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
- `jackdaw::dsl`, the Serverless Workflow 1.0.3 types, vendored from the Rust SDK (`just update-dsl` refreshes it).
- A guide in the rustdoc (`jackdaw::guide`) and runnable examples.
- `jackdaw visualize` now renders a workflow, and with `-i` the task states of a stored run.

### Changed

- **Breaking:** the project is a workspace of three crates. The library is published as `jackdaw-wf` (the name
  `jackdaw` on crates.io belongs to another project) and is still imported as `jackdaw`.
- **Breaking:** the YAML interpreter's types (`DurableEngine`, `DurableEngineBuilder`, the expression and context
  modules) are private; YAML workflows run through `Engine`.
- The YAML interpreter uses the engine's clock, saves a checkpoint when an instance starts, stores failures, and stops
  its runs when the engine is dropped.
- YAML HTTP and gRPC listeners fail the workflow when they cannot bind their port (before, the error was only logged),
  and the HTTP listener uses axum 0.8.
- `jackdaw run` prints its result when the output is not a terminal.
- The minimum supported Rust version is 1.90.

### Removed

- The Python bindings.
- `serverless_workflow_core` as a git dependency (its models are vendored as `jackdaw::dsl`).

## [0.1.1] - 2025-12-31

The CLI release before the library refactor; see the GitHub release.

[Unreleased]: https://github.com/arminhammer/jackdaw/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/arminhammer/jackdaw/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/arminhammer/jackdaw/releases/tag/v0.1.1
