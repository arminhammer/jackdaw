# jackdaw library API (design draft)

Status: draft. Decisions marked **Decided** were made on 2026-10-13. Everything else is a proposal to review.
The test suite in `tests/rust/` is written against this API (the target), not against the API as it is today.

## Goals
- Strong types for the input and output of every **task** and every **workflow**, checked by the compiler.
- One macro pattern for everything: `#[task]` for tasks, `#[workflow]` for workflows. Control flow is ordinary Rust.
- Workflows nest and are reused: any workflow can be run from another workflow.
- Tasks and workflows can be imported from a **catalog**, and an entire workflow can be defined in **pure Rust**.
- Rust and YAML workflows share one engine, one event log, one persistence layer, and can call each other.
- The engine is silent. It never prints; it emits events and `tracing` spans. The CLI does the printing.
- Everything a user must name is reachable from the crate root / `jackdaw::prelude`.

## Part 1: engine, handles, errors

### Quickstart (declarative, target)
```rust
use jackdaw::{Engine, Workflow};

let engine = Engine::builder().build()?;                      // in-memory persistence and cache
let workflow = Workflow::<Name, Greeting>::from_yaml(include_str!("hello.sw.yaml"))?;
let greeting: Greeting = engine.execute(&workflow, Name { name: "Ada".into() }).await?.await?;
```

### 1. A faulted workflow is an `Err` (**Decided**)
`run.await` returns `Result<Out, jackdaw::Error>`, where `Out` is the workflow's output type.
```rust
#[non_exhaustive]
pub enum Error {
    Faulted(Fault),         // raise / unhandled task error; carries the RFC 7807 fields
    Cancelled,
    Timeout { .. },
    Invalid(..),            // workflow failed validation (graph, DSL, unresolved call, type mismatch)
    NonDeterminism(..),     // a Rust workflow's steps diverged from its history on resume (Part 2)
    Persistence(..), Cache(..), Executor(..),   // engine/infra failures, with `source()`
}
pub struct Fault { pub kind: String /* error type URI */, pub status: Option<u16>,
                   pub title: Option<String>, pub detail: Option<String>, pub instance: Option<String> }
```
One public error type replaces the six per-module enums. `Error: std::error::Error + Send + Sync + 'static`.

### 2. `ExecutionHandle` and `execute` keep their names (**Decided**)
```rust
let run = engine.execute(&workflow, input).await?;  // returns as soon as the instance is created
run.id();                                           // &str, the instance id
let mut events = run.events();                      // EventStream (concrete type); may be called more than once
while let Some(event) = events.next().await { .. }  // `async fn next(&mut self) -> Option<Event>`, ends after the terminal event
run.cancel().await?;                                // &self
let output = run.await?;                            // ExecutionHandle<Out>: IntoFuture<Output = Result<Out, Error>>
```
Events and cancellation keep working while someone awaits. A deadline is `tokio::time::timeout(d, run)`; the engine's
`Timeout` error is for workflow- and task-level timeouts.

### 3. `jackdaw::Workflow<In, Out>` wraps the SDK type (**Decided**, generics added)
```rust
Workflow::<In, Out>::from_yaml(&str)? / from_json / from_path / from_definition(WorkflowDefinition)
workflow.name(), .namespace(), .version(), .definition() -> &WorkflowDefinition
pub use serverless_workflow_core as models;
```
`In`/`Out` are `Serialize`/`DeserializeOwned` Rust types, and the constructors have no default for them: the types are
written, or inferred from use. Binding a YAML workflow to types is checked against its `input.schema`/`output.schema` when
it has them; otherwise it is trusted and a mismatch becomes a `validation` fault at run time (the same guarantee as
`serde_json::from_str::<T>`; only workflows written in Rust are checked by the compiler).

### 4. Typing (**Decided**, revised)
Everything user-facing is typed: tasks, workflows, `ExecutionHandle<Out>`. There is no named untyped workflow and no default
type parameters. Code that genuinely cannot know a workflow's shape (the CLI, an admin tool) says so with
`Workflow::<serde_json::Value, serde_json::Value>`, which is an ordinary typed workflow whose types happen to be `Value`.

### 5. `DurableEngine` is renamed `Engine` (**Decided**)
`Engine` replaces `DurableEngine`; `DurableEngineBuilder` becomes `EngineBuilder`. The old names stay as `#[deprecated]`
aliases for one release.

## Part 2: typed tasks and workflows with macros (proposal)

Supersedes the earlier builder DSL (`Flow`/`Var`), which was chained function calls. The pattern below is modelled on
Temporal's Rust SDK, with jackdaw's own vocabulary.

### Vocabulary
- **Task**: a unit of work with typed input and output: a native Rust function, a built-in call (`http`, `openapi`, ...),
  or a catalog task. It is what the spec calls a function, callable from YAML with `call: name`.
- **Workflow**: Rust code (or YAML) that runs tasks and other workflows. Typed input and output.
- **Catalog**: a source of versioned tasks and workflows (the spec's `use.catalogs`).
There is no "activity" concept.

### A. Tasks
```rust
#[derive(Serialize, Deserialize)] pub struct ChargeRequest { pub amount_cents: u64, pub currency: String }
#[derive(Serialize, Deserialize)] pub struct Receipt { pub id: String }

#[task]                                                   // task name: "charge"
async fn charge(ctx: TaskContext, req: ChargeRequest) -> Result<Receipt, TaskError> { /* ... */ }

// Tasks with state: an impl block
pub struct PaymentTasks { gateway: Arc<Gateway> }

#[tasks]
impl PaymentTasks {
    #[task]
    pub async fn refund(self: Arc<Self>, _ctx: TaskContext, r: Receipt) -> Result<(), TaskError> { /* ... */ }
}
```
`#[task]` keeps the function and generates a typed marker (`Charge`, `PaymentTasks::refund`) that carries the input and
output types. The context argument is optional; `#[task(blocking)]` runs sync/CPU-bound code on a blocking thread.

### B. Workflows
```rust
#[derive(Serialize, Deserialize)] pub struct OrderInput  { pub order_id: String, pub total_cents: u64, pub items: Vec<Item> }
#[derive(Serialize, Deserialize)] pub struct OrderOutput { pub receipt: Receipt, pub shipment: Shipment }

#[workflow(namespace = "shop", name = "order", version = "1.0.0")]
pub struct OrderWorkflow { approved: bool }               // workflow state (fields), used by signals and queries

#[workflow_methods]
impl OrderWorkflow {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self { Self { approved: false } }

    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: OrderInput) -> WorkflowResult<OrderOutput> {
        let valid = ctx.call(Validate, &input).await?;                      // Validate: OrderInput -> Validated

        if valid.total_cents > 100_000 {
            ctx.wait_condition(|s| s.approved).await?;                      // until the `approve` signal arrives
        }

        let priced = ctx.for_each(&valid.items, PriceItem).await?;          // Vec<Priced>; the engine's `for` task

        let receipt = match ctx.call(Charge, ChargeRequest::from(&priced))
            .timeout(30.secs()).retry(Retry::exponential(5)).await
        {
            Ok(receipt) => receipt,
            Err(StepError::Fault(f)) if f.status == Some(402) => {
                ctx.call(Notify, &f).await?;
                return Err(f.into());
            }
            Err(e) => return Err(e.into()),
        };

        let shipment = ctx.run(ShipWorkflow::run, ShipInput::new(&valid, &receipt)).await?;   // nested workflow, typed
        Ok(OrderOutput { receipt, shipment })
    }

    #[signal]
    fn approve(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) { self.approved = true; }

    #[query]
    fn status(&self, _ctx: &WorkflowContextView) -> OrderStatus { /* ... */ }
}
```
- Control flow is plain Rust: `if`, `match`, loops, `?`, early `return`. Errors from a step are a typed `StepError` you can
  match; returning it faults the workflow as `Error::Faulted(Fault)` with the spec's error types.
- `#[run]` takes the typed input and returns `WorkflowResult<Out>`; the macro derives `In`/`Out` from its signature and
  generates a typed marker (`OrderWorkflow`) that carries them.
- A workflow with no state is a unit struct and needs no `#[init]`. Signals, queries and updates are optional.
- `ctx.for_each(&items, Task)` is the engine's `for` task; `futures::join!(a, b)` / `join_all` run steps in parallel (the
  engine's `fork`); `jackdaw::select!` is `fork` with `compete`. They return typed tuples/vectors.
- Step options chain on any step: `.timeout(d)`, `.retry(policy)` (compiled to the engine's `try` with `catch.retry`),
  `.cached()`, `.input(..)`/`.output(..)` transforms for the spec's data-flow expressions.

### C. Hosting and starting
```rust
let engine = Engine::builder()
    .store(Redb::open("jackdaw.db")?)
    .task(Charge).tasks(PaymentTasks { gateway })
    .workflow::<OrderWorkflow>().workflow::<ShipWorkflow>()
    .catalog("local", Catalog::file("./catalog"))
    .build()?;

let run = engine.execute(OrderWorkflow::run, OrderInput { .. }).await?;      // ExecutionHandle<OrderOutput>
run.signal(OrderWorkflow::approve, ()).await?;                               // argument type comes from the handler
let status: OrderStatus = run.query(OrderWorkflow::status).await?;
let out: OrderOutput = run.await?;
```
The macros generate markers (`OrderWorkflow::approve`, `PaymentTasks::refund`) that carry types, so starts, signals,
queries, calls and nested runs are all compiler-checked. No strings and no `serde_json::Value` at the call site.

### D. Nesting and reuse
A workflow is referenced by its `#[run]` method (**Decided**), the same way signals and queries are: `OrderWorkflow::run`,
`OrderWorkflow::approve`. The macro renames the method you wrote and emits a typed const of that name, because a struct with
fields cannot be used as a value (`engine.execute(Approval, ..)` cannot compile). A run marker is usable wherever a task is:
`ctx.run(ShipWorkflow::run, input)` returns `ShipWorkflow`'s output type. Child
workflows can be Rust, YAML bound to types (`Workflow::<In, Out>::from_yaml(..)`, `ctx.run_definition(&wf, input)`) or
catalog workflows. Registering a workflow on the engine registers its children's markers too; the engine's existing
`register_workflow` registry (keyed by `namespace/name/version`) sits underneath. YAML can call any registered Rust **task**
with plain `call: charge` (resolution: workflow `use.functions` -> engine task table -> built-in call types -> catalog).
A YAML workflow cannot run a Rust **workflow** as a child (**Decided**, 2026-10-13: dropped; it needed a hook into the YAML
engine for little value). Interop is one direction for workflows: Rust runs YAML. Recursion is allowed only through the
engine's normal depth limit.

### E. Catalogs
Imports use the same macro style: declare the entries as a trait, with types.
```rust
#[catalog(source = "file://./catalog")]                    // also http(s)://..., git://...
trait LocalCatalog {
    #[task(name = "add-numbers", version = "1.0.0")]
    async fn add(input: AddIn) -> Result<AddOut, TaskError>;

    #[workflow(name = "kyc", version = "2.1.0")]
    async fn kyc(person: Person) -> WorkflowResult<KycResult>;
}

// in a workflow:
let sum = ctx.call(LocalCatalog::add, AddIn { a: 1, b: 2 }).await?;
let kyc = ctx.run(LocalCatalog::kyc, &person).await?;
```
The macro generates markers (`LocalCatalog::add`, `LocalCatalog::kyc`) that compile to `call: add-numbers:1.0.0` /
`run.workflow`, resolved by the engine's existing catalog loader (`durableengine/catalog.rs`). The types are yours. They
are verified when the engine is built: a missing entry or version is `Error::Invalid` immediately; if the catalog entry
carries `input.schema`/`output.schema`, they are checked against the Rust types' schemas (needs `schemars::JsonSchema`);
otherwise the binding is trusted and a mismatch is a `validation` fault. An optional `jackdaw-codegen` can generate the
trait and types from a catalog's schemas. Catalog entries stay plain spec YAML, usable by other runtimes.

### F. The step vocabulary reuses the engine's task types
| `ctx` method | Engine task it runs | Existing implementation |
|---|---|---|
| `ctx.call(Task, input)` | `call` (registered function) | `tasks/call.rs` |
| `ctx.http()`, `ctx.openapi()`, `ctx.grpc()` | `call` http / openapi / grpc | `providers/executors/*` |
| `ctx.sleep(d)` / `ctx.sleep_until(t)` | `wait` | `tasks/wait.rs` |
| `ctx.emit(event)` | `emit` | `tasks/emit.rs` |
| `ctx.wait_for_event::<E>(filter)`, signals | `listen` | listen task |
| `ctx.run_container/shell/script(..)` | `run` | `tasks/run.rs` |
| `ctx.run(Workflow, input)` / `run_definition` | `run.workflow` (child) | `tasks/run.rs` |
| `ctx.for_each(&items, Task)` | `for` | `tasks/for_loop.rs` |
| `futures::join!(..)` / `jackdaw::select!{..}` | `fork` / `fork` + `compete` | `tasks/fork.rs` |
| `return Err(Fault::..)` | `raise` | `tasks/raise.rs` |
| `if` / `match` / `?` / blocks | `switch` / `try` / `do` | native Rust |

Every step goes through the engine's existing executors, so retries, task timeouts, events and persistence behave as
they do for YAML workflows.

### G. How a Rust workflow runs: replay, made ergonomic
`#[run]` contains arbitrary Rust control flow, so the engine makes it durable by journaling each step and, when an
instance has to be rebuilt, running `#[run]` again from the top: finished steps return their recorded result, and the
first unrecorded step runs live. The journal is the engine's existing `TaskCreated` / `TaskStarted` / `TaskCompleted` /
`TaskFaulted` events in the `PersistenceProvider`, plus a step key and call site. The goal of everything below is that you
rarely have to think about replay: it is rare, the compiler enforces the rules, and code changes are survivable.

**1. Replay is the exception.** In a healthy process `#[run]` is an ordinary future that simply keeps running; each step
result is written to the journal as it completes. Replay happens only when an instance is rehydrated: after a crash or
restart, or when a parked long `ctx.sleep`/`wait_for_event` fires. Replayed steps return recorded results without
re-executing and without re-emitting events (`run.events()` shows each step once; replayed events carry `replayed: true`).

**2. A step's identity is assigned when you call it, not when it is polled.** `ctx.call(..)` takes its identity
synchronously, in program order. So the standard `futures::join!`, `try_join!` and `join_all` are safe and need no special
combinators. Only a *race* needs `jackdaw::select!` (it journals the winner). Bounded parallelism is
`ctx.for_each(&items, Task).concurrency(8)`; `buffer_unordered` over `ctx` calls creates steps in completion order and is
rejected by the lint below.

**3. The compiler enforces the rules (**Decided**: a hard error).** `#[workflow_methods]` scans each `#[run]` body and
fails the build, at the offending span and with the fix, for the common non-deterministic calls: `SystemTime::now`/
`Instant::now` (use `ctx.now()`), `rand::random`/`thread_rng` (`ctx.random()`), `Uuid::new_v4` (`ctx.uuid()`),
`tokio::spawn`, `thread::sleep`, `tokio::time::sleep`/`timeout` (`ctx.sleep()`/`.timeout()`), `tokio::select!`
(`jackdaw::select!`), and `std::fs`/`std::net`/`std::env` and HTTP clients (put the call in a `#[task]`).
`WorkflowContext` is only ever borrowed, so a spawned future cannot capture it. The error can be silenced for a statement
with `#[allow_nondeterminism]`, which is meant for code you have audited. (It is a plain attribute rather than
`#[allow(jackdaw::nondeterministic)]` because rustc rejects an unknown tool in `allow` before any attribute macro can
remove it; this was checked.) The scan catches the common mistakes, not every one; self-replay in tests (7) catches the rest.

**4. Side effects are tasks (**Decided**).** There is no inline escape hatch: anything that touches the outside world is a
`#[task]`, which is one attribute on an ordinary function and can sit right next to the workflow in the same module. That keeps
the boundary visible, makes every effect testable and mockable by name, and gives retries, timeouts, events and caching a
single place to attach. `ctx.now()`, `ctx.random()` and `ctx.uuid()` are journaled values, not side effects.

**5. Code changes are survivable.**
- *Versions pin code.* A workflow's identity already includes `version` (`#[workflow(version = "1.0.0")]`). An instance
  resumes with the code registered for its version, so the safe default for a behaviour change is: bump the version, keep the
  old implementation registered while old instances drain (`.workflow::<OrderV1>().workflow::<OrderV2>()`). No replay
  compatibility is needed between versions.
- *Replay is strict by default (**Decided**).* A step's key is its global position plus its kind, task name and input hash.
  If the code issues a different step than the history recorded, the instance fails with `Error::NonDeterminism` rather than
  guessing. This is the safe behaviour for workflows with side effects: nothing runs against a history it does not match.
- *`ctx.patched("add-fraud-check")`* is how you change a workflow in place under strict replay: it returns `true` for new
  executions and `false` when replaying a history that started before the patch, so old runs keep following the old path.
  Prefer bumping the version when the change is large.
- *Lenient replay is an opt-in:* `#[workflow(replay = "lenient")]` keys steps by `(kind, task name, n-th use of that name)`
  instead, so adding, removing or reordering unrelated steps does not break replay. A step with no record runs live, a
  recorded step the code no longer issues is ignored with a warning, and a recorded step whose input changed returns the
  recorded result with a `replay_input_mismatch` event. Use it only for workflows whose steps are safe to re-run.
- *Check stored histories before you deploy:* `engine.replay_all(&store)` (or `jackdaw replay --store jackdaw.db`) replays
  every stored history against the current code and reports each divergence.

**6. Failures point at your code.** Steps record their call site with `#[track_caller]`, so a divergence reads:
`NonDeterminism: step "charge" #1 at src/order.rs:42 expected Charge, history has Notify` with the surrounding history.

**7. Tests get replay for free.** `jackdaw::testing::TestEngine` runs every workflow twice by default: live, then replayed
from the history it just produced, asserting the same outcome. That catches non-determinism in an ordinary test with no
extra code. Disable with `.self_replay(false)`.

Tasks are **at-least-once**: one running when the process died runs again; `ctx.idempotency_key()` is stable across
retries and replay. **No caching by default for native tasks.** Today `exec_call_task` caches every call by
`(task_name, params)` and ignores the instance, which would skip real side effects; `#[task(cached)]` opts in (a
`cacheable()` capability on `Executor`).

### H. Errors and context
`TaskError`: `TaskError::retryable(e)`, `TaskError::non_retryable("..")`, `.next_retry_delay(d)`, or `?` on any
`std::error::Error` (a `runtime` fault). It becomes a spec fault (type, status, title, detail, instance) that a workflow
sees as `StepError::Fault(Fault)` and YAML `try/catch` can filter by type. `TaskContext`: `instance_id()`, `task_name()`,
`attempt()`, `cancelled()`, `emit(impl Serialize)`, `state::<T>()` (set with `EngineBuilder::state(T)`),
`idempotency_key()`. Panics in tasks are caught and reported as `runtime` faults; a panic in `#[run]` fails that workflow only.

### I. Messages: signals, queries, updates
`#[signal]` is fire-and-forget and mutates state through `&mut self`, delivered through the engine's event path (the same
mechanism as `listen` and the HTTP/gRPC listeners). `#[query]` is read-only, answered from current state, never recorded.
`#[update]` is a signal that returns a typed result.

### J. Testing
`jackdaw::testing::TestEngine`: in-memory store, task mocks (`.mock(Charge, |req| Ok(receipt))`), a virtual clock so
`ctx.sleep` doesn't wait, `replay(history, workflow)`, and `golden(..)` helpers. These are also the basis of `tests/rust/`.

### Risks (spike first)
1. The engine's task executors take an internal `Context` built around a `WorkflowDefinition`. A Rust workflow needs a
   synthesized minimal definition and an adapter so steps can run outside the graph executor.
2. Step identity at call time must hold for every combinator we advertise (`join!`, `join_all`, `try_join!`); `select!`
   must journal its winner; the lint (G.3) will have false negatives.
3. Signals and queries need a per-instance mailbox and an engine API to deliver messages to a running instance.
4. A new proc-macro crate (`jackdaw-macros`) joins the workspace.
5. The event log gains a few fields (step id); the persistence providers must stay compatible.
6. A Rust workflow cannot be exported to YAML (it is code). The reverse works: YAML can call its tasks and run it as a child.

### Suggested build order
- **P0 spike:** `ctx.call` and `ctx.sleep` journaled through the existing executors, one `#[task]`, one `#[workflow]`; a test
  that kills the engine mid-workflow, reopens the store and resumes to completion; then nest it in a second workflow.
- **P1:** the macros end to end, typed `execute`/`ExecutionHandle<Out>`, `NonDeterminism`, `Engine::builder().task/.workflow`.
- **P2:** the rest of the step table (`http`, `emit`, `run_*`, `for_each`, `select!`), step options, and the
  determinism lint with its compile-fail tests (cheap to build early, and it shapes how people write workflows).
- **P3:** catalogs (`#[catalog]`, schema checks), YAML tasks callable from Rust and Rust tasks callable from YAML.
- **P4:** signals, queries, updates, `wait_condition`; then `TestEngine` (with self-replay), mocks.
- **P5:** replay-compatibility tooling: `replay_all` / `jackdaw replay`, `ctx.patched`, the lenient opt-in, call-site diagnostics.

## Part 3: other proposals (not yet decided)

### Durability API
```rust
let store = Redb::open("jackdaw.db")?;                      // one handle, implements both traits
let engine = Engine::builder().store(store).build()?;       // or .persistence(p).cache(c) separately
let run = engine.execute_with_id("order-42", OrderWorkflow::run, input).await?; // idempotent: same id, same instance
let run = engine.resume::<OrderWorkflow>("order-42").await?;               // an ExecutionHandle, like execute
let status = engine.status("order-42").await?;                             // Running | Completed | Faulted | Cancelled | Suspended
```
`resume` returns an `ExecutionHandle` (today it returns a bare `Value`). `PersistenceProvider` and `CacheProvider` stay
`Arc<dyn Trait>` internally; the builder accepts `impl Trait + 'static`.

### Extension
`Engine::builder().executor("my-call", MyExecutor)` registers a custom call type. The `Executor` trait loses `as_any` and
the console-output `TaskOutputStreamer`; it receives the evaluated `with` parameters plus an `ExecContext`. Built-in
executors are registered the same way. `#[task]` is the typed layer over this.

### Events
`Event` is a public, `#[non_exhaustive]` enum (today's `WorkflowEvent`): `WorkflowStarted`, `TaskStarted`, `TaskCompleted`,
`TaskFaulted`, `TaskRetried`, `WorkflowCompleted`, `WorkflowFailed`, `WorkflowCancelled`, ... The stream ends after the
terminal event.

## Open questions
Resolved (2026-10-13): strict replay is the default (G.5); the determinism lint is a hard error (G.3); side effects are
`#[task]`s (G.4); the `#[workflow]` + `#[workflow_methods]` split stays; the `#[catalog] trait` form stays; `execute` and
`ExecutionHandle` keep their names; the event stream is a concrete `EventStream` with `async fn next()`; development is
test-first, so `tests/rust/` is written against this document before the implementation exists.
- `#[run]` takes the input directly (as drafted); the tests assume this unless you say otherwise.
- Where do listeners live (on `Engine`, or an `engine.listeners()` sub-API)? Not covered by the first test pass.

## Mapping to the test suite (`tests/rust/`)
The suite exists and is the executable form of this document (it does not compile until the library catches up). The exact
signatures it pins down, including some this document does not spell out (`run_task`, `Status`, `history`, `replay_all`,
`.catalog::<C>()`, `send_event`, `TestEngine`, `Event::name/replayed`), are listed in `jackdaw/tests/rust/README.md`.
Where the two disagree, fix the one that is wrong. The modules in the suite are `quickstart`, `builder`, `tasks`,
`rust_workflows`, `nesting`, `catalogs`, `replay`, `events`, `errors`, `api_surface` and `compile_fail`; the table below
is the original plan and is only a rough guide.

| Module | Pins down |
|---|---|
| `builder` | `Engine::builder()`, defaults, `.store/.persistence/.cache/.task/.tasks/.workflow/.catalog`, event buffer |
| `workflows` | `Workflow::<In, Out>::from_*`, validation errors (declarative path) |
| `execute` | `execute`, awaiting, DSL task types, faults as `Err(Faulted)` |
| `events` | `run.events()` ordering, terminal event, concurrent use with `await` |
| `cancel` | `run.cancel()` while awaiting, `Err(Cancelled)`, stable `id()` |
| `durability` | `execute_with_id`, `resume`, `status` |
| `tasks` | `#[task]`/`#[tasks]`, typed in/out, `TaskError` kinds -> faults, retries/timeouts, panics, no-cache default |
| `rust_workflows` | `#[workflow]`/`#[run]`, steps in the table above, `join!`/`select!`, `StepError` matching |
| `nesting` | Rust, YAML-bound and catalog children; Rust runs YAML (YAML does not run Rust workflows) |
| `catalogs` | `#[catalog]`, typed markers, schema checks, missing entry/version |
| `replay` | kill/resume mid-workflow, strict keys (default) vs lenient opt-in, `patched`, `replay_all`, self-replay in `TestEngine`, `NonDeterminism` message with call site, `idempotency_key` stability, events not duplicated on replay, `ctx.now/random/uuid` stable across replay |
| `determinism_lint` | compile-fail tests (`trybuild`) for each forbidden call, its suggested fix, and the `#[allow_nondeterminism]` opt-out |
| `messages` | signals, queries, updates, `wait_condition` |
| `testing` | `TestEngine`, mocks, virtual clock |
| `errors` | `Error` variants are matchable; `source()` chains; `Send + Sync + 'static` |
| `api_surface` | everything above compiles from `use jackdaw::prelude::*` alone |
