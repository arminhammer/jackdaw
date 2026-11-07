# jackdaw-wf

Durable workflows in Rust. Write workflows as ordinary async functions, or in the
[Open Workflow](https://open-workflow-specification.org/) YAML DSL (formerly Serverless Workflow), and run both on one engine that records every step, so
a workflow that is interrupted continues where it stopped.

- **Strongly typed.** Tasks and workflows have typed inputs and outputs, checked by the compiler. Signals, queries,
  child workflows and catalog imports are typed the same way.
- **Plain Rust.** Control flow is `if`, `match`, loops and `?`. Macros name things; they do not define a DSL.
- **Durable.** Steps are journaled. A crashed or restarted process resumes its workflows from the journal, and
  `Engine::replay_all` tells you before you deploy whether a change would break them.
- **Checked at compile time.** Calls that would make a workflow non-deterministic, like `SystemTime::now()`, are
  compile errors with the fix in the message.
- **One engine for Rust and YAML.** They share events and storage and can call each other: Rust runs YAML workflows,
  and YAML calls Rust tasks.
- **Testable.** `testing::TestEngine` runs workflows on a virtual clock and replays each run to verify it.

## Install

```sh
cargo add jackdaw-wf --features cli
cargo add serde --features derive
cargo add tokio --features macros,rt-multi-thread
```

The package is `jackdaw-wf` on crates.io, and the library is imported as `jackdaw`.

## A workflow in a minute

A workflow's input is its configuration: one struct that says everything a run needs. `#[derive(Config)]` turns it into
command line flags, and the same fields can be read from environment variables and files.

```rust,no_run
use jackdaw::prelude::*;
use jackdaw::runner::Program;
use serde::{Deserialize, Serialize};

// The workflow's input describes everything a run needs. `Config` turns it into command line flags; the same
// fields can come from `ORDER_`-prefixed environment variables or from `--config order.toml`.
#[derive(Serialize, Deserialize, Config)]
struct Order {
    /// The order to ship
    id: String,
    /// What to charge, in cents
    cents: u64,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Shipment {
    tracking: String,
}

// A task is a typed function with side effects. Its result is recorded. `#[task]` also creates a
// marker named `Charge` that carries the types, which is what the workflow and the engine refer to.
#[task]
async fn charge(order: Order) -> Result<String, TaskError> {
    Ok(format!("ch-{}", order.id))
}

#[task]
async fn ship(charge_id: String) -> Result<Shipment, TaskError> {
    Ok(Shipment { tracking: format!("trk-{charge_id}") })
}

// A workflow decides which tasks run. It is deterministic, and it can be resumed.
#[workflow(namespace = "shop", name = "order", version = "1.0.0")]
struct OrderWorkflow;

#[workflow_methods]
impl OrderWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, order: Order) -> WorkflowResult<Shipment> {
        let charge_id = ctx
            .call(Charge, &order)
            .retry(Retry::exponential(5))
            .timeout(30.secs())
            .await?;
        Ok(ctx.call(Ship, &charge_id).await?)
    }
}

/// Run it as `order --id o-1 --cents 500`; `order --help` lists the flags. The same input can come from
/// `ORDER_ID=o-1 ORDER_CENTS=500 order` or from `order --config order.toml`.
#[tokio::main]
async fn main() -> Result<std::process::ExitCode, Error> {
    let engine = Engine::builder()
        .task(Charge)
        .task(Ship)
        .workflow::<OrderWorkflow>()
        .build()?;
    // Reads the input, runs the workflow, shows its progress and prints the shipment as JSON.
    Ok(Program::<OrderWorkflow>::new(engine).main().await)
}
```

Add `.store(Redb::open("jackdaw.redb")?)` to the builder and the workflow survives the process.

## Where to go next

The [guide](https://docs.rs/jackdaw-wf/latest/jackdaw/guide/) walks through the library chapter by chapter: tasks,
workflows, durability, catalogs, YAML, testing, errors and events. The reference lives in
[`api`](https://docs.rs/jackdaw-wf/latest/jackdaw/api/) and [`prelude`](https://docs.rs/jackdaw-wf/latest/jackdaw/prelude/).

The [`examples`](https://github.com/arminhammer/jackdaw/tree/main/jackdaw/examples) directory has runnable programs:
`cargo run --example quickstart`.

## Features

| Feature | Default | What it adds |
|---|---|---|
| `yaml` | yes | The Open Workflow interpreter: `dsl`, `api::Workflow` (`from_yaml`, `validate`), `ctx.run_definition`, catalogs (`#[catalog]`). Includes `http-client`. |
| `redb` | yes | The `Redb` store. |
| `http-client` | yes | `ctx.http`, YAML `call: http`, remote catalogs. |
| `sqlite`, `postgres` | | The `Sqlite` and `Postgres` stores. |
| `docker` | | `ctx.run_container` and YAML `run: container`. |
| `openapi` | | YAML `call: openapi`. |
| `http-listener`, `grpc-listener` | | `api::Listener`: the engine serves an OpenAPI document's operations or a gRPC service, each request starting a workflow. Also YAML `listen` tasks on HTTP and gRPC. |
| `config` | | `config::Layers` and `#[derive(Config)]`: a workflow's input from defaults, files (toml, json, yaml), environment variables and flags. |
| `terminal` | | `runner::Terminal`: progress bars for a run in a terminal. |
| `cli` | | `runner::Program` and `RunnerArgs`: a workflow as a program with flags (includes `config` and `terminal`). |
| `metrics` | | Counters and histograms through the `metrics` facade. |
| `full` | | All of the above. |

Without `yaml`, the native Rust API (tasks, workflows, the engine, the in-memory store, `testing`) still works:
`default-features = false` builds about 70 crates instead of about 150. A YAML workflow that needs a feature the build
lacks, such as a `listen` task without `http-listener`, fails with an error that names the feature.

The library also builds for WebAssembly (`wasm32-wasip2`, for wasmtime, Spin and other WASI hosts): the native API, the
YAML interpreter and the Redb store work there, and what needs a process, a socket or an HTTP client does not. See the
[guide](https://docs.rs/jackdaw-wf/latest/jackdaw/guide/wasm/).

## Command line

The `jackdaw-cli` crate installs a `jackdaw` binary that runs, validates and visualizes YAML workflows:

```sh
cargo install jackdaw-cli --locked
jackdaw run workflow.yaml
jackdaw validate workflow.yaml
```

## License

Apache-2.0. `src/dsl` is vendored from the [Open Workflow Specification Rust SDK](https://github.com/open-workflow-specification/sdk-rust),
also Apache-2.0; see `src/dsl/LICENSE`.
