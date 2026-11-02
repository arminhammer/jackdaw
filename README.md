# jackdaw

[![crates.io](https://img.shields.io/crates/v/jackdaw-wf.svg)](https://crates.io/crates/jackdaw-wf)
[![docs.rs](https://img.shields.io/docsrs/jackdaw-wf)](https://docs.rs/jackdaw-wf/latest/jackdaw/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Durable workflows in Rust. Write workflows as ordinary async Rust functions, or in the
[Open Workflow](https://open-workflow-specification.org/) YAML DSL (formerly Serverless Workflow), and run both on one
engine. The engine records every step, so a workflow that is interrupted, by a crash, a deploy or a restart,
continues where it stopped instead of starting over.

jackdaw is a library first: there is no server to install. Embed the engine in your service, or use the `jackdaw`
command-line tool to run YAML workflows directly.

## Crates

| Crate | What it is | Docs |
|---|---|---|
| [`jackdaw-wf`](jackdaw) | The library, imported as `jackdaw`: the engine, the Rust API, the stores and the YAML interpreter | [README](jackdaw/README.md), [docs.rs](https://docs.rs/jackdaw-wf/latest/jackdaw/) |
| [`jackdaw-wf-macros`](jackdaw-macros) | `#[task]`, `#[workflow]` and the other macros, re-exported by `jackdaw-wf` | [README](jackdaw-macros/README.md) |
| [`jackdaw-cli`](jackdaw-cli) | The `jackdaw` binary: run, validate and visualize YAML workflows | [README](jackdaw-cli/README.md) |

All three are released together, at the same version.

## Highlights

- **Durable.** Each step is journaled. A restarted process resumes its workflows from the journal, and completed
  steps are not run again.
- **Typed.** Tasks, workflows, signals, queries and child workflows have typed inputs and outputs, checked by the
  compiler.
- **Plain Rust.** Control flow is `if`, `match`, loops and `?`. The macros name things; they do not define a DSL.
- **Deterministic by construction.** Calls that would break replay, like `SystemTime::now()`, are compile errors
  whose message names the replacement.
- **One engine for Rust and YAML.** They share events and storage, and they call each other: Rust runs YAML
  workflows, and YAML calls Rust tasks.
- **Operational features.** Retries, timeouts, heartbeats, caching, durable timers, cron schedules, leases for
  several engines on one store, retention, `tracing` spans and `metrics`.
- **Pluggable storage.** Redb (embedded, the default), SQLite and Postgres, or your own store.
- **Serve APIs from workflows.** Listeners start a workflow for each request to an OpenAPI operation or a gRPC
  method, with requests checked against the OpenAPI document.
- **Testable.** `testing::TestEngine` runs workflows on a virtual clock and replays each run to check that it is
  deterministic.

## Quick start: Rust

```sh
cargo add jackdaw-wf serde --features serde/derive
cargo add tokio --features macros,rt-multi-thread
```

```rust
use jackdaw::prelude::*;

// A task does the work. Its result is recorded, so it is not repeated after a restart.
#[task]
async fn greet(name: String) -> Result<String, TaskError> {
    Ok(format!("Hello, {name}!"))
}

// A workflow decides which tasks run. It is deterministic, so it can be replayed from its journal.
#[workflow(namespace = "examples", name = "hello", version = "1.0.0")]
struct Hello;

#[workflow_methods]
impl Hello {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, name: String) -> WorkflowResult<String> {
        Ok(ctx.call(Greet, &name).retry(Retry::exponential(3)).await?)
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder()
        .store(Redb::open("jackdaw.redb")?) // without a store, state stays in memory
        .task(Greet)
        .workflow::<Hello>()
        .build()?;
    engine.recover().await?; // resume whatever was running when the last process stopped

    let greeting = engine.execute(Hello::run, "World".to_string()).await?.await?;
    println!("{greeting}");
    Ok(())
}
```

The [library README](jackdaw/README.md) has a longer example, and the
[guide](https://docs.rs/jackdaw-wf/latest/jackdaw/guide/) covers tasks, workflows, durability, catalogs, YAML,
testing, errors and events.

## Quick start: YAML and the CLI

```sh
cargo install jackdaw-cli --locked   # or: cargo binstall jackdaw-cli
```

```yaml
# hello-world.sw.yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: hello-world
  version: '0.1.0'
output:
  as: .statement
do:
  - say:
      set:
        statement: Hello, World!
```

```sh
jackdaw run hello-world.sw.yaml            # prints "Hello, World!"
jackdaw run hello-world.sw.yaml --debug    # with a task-by-task trace
jackdaw validate hello-world.sw.yaml
```

Prebuilt binaries for Linux (amd64, arm64) and macOS are attached to each
[GitHub release](https://github.com/arminhammer/jackdaw/releases), and a Docker image is published as
`ghcr.io/arminhammer/jackdaw`. The [CLI README](jackdaw-cli/README.md) covers scripts, containers, nested workflows,
catalogs, caching, persistence and listeners, with examples in [`jackdaw-cli/examples`](jackdaw-cli/examples).

## Cargo features

The library's defaults are `yaml`, `redb` and `http-client`. Opt in to `sqlite`, `postgres`, `docker`, `openapi`,
`http-listener`, `grpc-listener` and `metrics` as needed, or `full` for all of them. With
`default-features = false`, the native Rust API still works and the build pulls in far fewer crates. The
[library README](jackdaw/README.md#features) lists what each feature adds.

## Repository layout

```text
jackdaw/          the library (jackdaw-wf): src/, examples/, and its test suites under tests/
jackdaw-macros/   the procedural macros (jackdaw-wf-macros)
jackdaw-cli/      the command-line tool (jackdaw-cli), its Dockerfile and examples
submodules/       the Open Workflow specification, for the conformance tests
```

## Development

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.99 for development); the minimum supported Rust version is
1.94. Common tasks have [`just`](https://github.com/casey/just) recipes:

```sh
just build         # build the workspace
just test-lib      # the library's executable specification (Gherkin features in jackdaw/tests/lib)
just test-all      # every test suite
just lint          # clippy with warnings denied
just deny          # licenses and dependency sources
just semver        # the public API against the latest release
```

[CONTRIBUTING.md](CONTRIBUTING.md) explains the test suites and their environment variables, the spec-driven
workflow, the vendored DSL models and the release process. Changes are listed in [CHANGELOG.md](CHANGELOG.md), and
security reports go through [SECURITY.md](SECURITY.md).

## License

Apache-2.0. `jackdaw/src/dsl` is vendored from the
[Open Workflow Specification Rust SDK](https://github.com/open-workflow-specification/sdk-rust), also Apache-2.0.
