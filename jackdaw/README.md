# jackdaw

Durable workflows in Rust. Write workflows as ordinary async functions, or in the
[Serverless Workflow](https://serverlessworkflow.io/) YAML DSL, and run both on one engine that records every step, so
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
cargo add jackdaw serde --features serde/derive
cargo add tokio --features macros,rt-multi-thread
```

## A workflow in a minute

```rust
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Order {
    id: String,
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

#[tokio::main]
async fn main() -> Result<(), Error> {
    let engine = Engine::builder()
        .task(Charge)
        .task(Ship)
        .workflow::<OrderWorkflow>()
        .build()?;

    let shipment = engine
        .execute(OrderWorkflow::run, Order { id: "o-1".into(), cents: 500 })
        .await?
        .await?;
    assert_eq!(shipment.tracking, "trk-ch-o-1");
    Ok(())
}
```

Add `.store(Redb::open("jackdaw.redb")?)` to the builder and the workflow survives the process.

## Where to go next

The [guide](https://docs.rs/jackdaw/latest/jackdaw/guide/) walks through the library chapter by chapter: tasks,
workflows, durability, catalogs, YAML, testing, errors and events. The reference lives in
[`api`](https://docs.rs/jackdaw/latest/jackdaw/api/) and [`prelude`](https://docs.rs/jackdaw/latest/jackdaw/prelude/).

The [`examples`](https://github.com/arminhammer/jackdaw/tree/main/jackdaw/examples) directory has runnable programs:
`cargo run --example quickstart`.

## Command line

The `jackdaw-cli` crate builds a `jackdaw` binary that runs, validates and visualizes YAML workflows:

```sh
jackdaw run workflow.yaml
jackdaw validate workflow.yaml
```

## License

Apache-2.0. `src/dsl` is vendored from the [Serverless Workflow Rust SDK](https://github.com/serverlessworkflow/sdk-rust),
also Apache-2.0; see `src/dsl/LICENSE`.
