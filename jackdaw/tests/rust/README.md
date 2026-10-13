# Library API tests (`tests/rust`)

Written test-first against [`../../API.md`](../../API.md). **This suite does not compile yet, on purpose**: it
describes the jackdaw library API we want, and each piece of the library should arrive by making part of it compile
and then pass. `cargo build --all-targets`, `just lint` and `cargo test --workspace` fail on this branch until then.
Run the other suites with `--test <name>` (for example `cargo test -p jackdaw --test ctk_conformance`).

```
cargo test -p jackdaw --test rust                 # everything
cargo test -p jackdaw --test rust tasks::         # one module
TRYBUILD=overwrite cargo test -p jackdaw --test rust compile_fail   # write/refresh ui/*.stderr
```

## Reading order
1. `quickstart.rs` — the two ways to run something: a YAML workflow bound to types, and a workflow written in Rust.
2. `tasks.rs` — `#[task]`, errors, retries, timeouts, caching.
3. `rust_workflows.rs` — `#[workflow]`: control flow, parallelism, signals, queries, waiting.
4. `nesting.rs`, `catalogs.rs` — reuse: child workflows (Rust runs Rust or YAML), `#[catalog]`.
5. `replay.rs` — durability, evolving code, `NonDeterminism`.
6. `events.rs`, `errors.rs`, `builder.rs`, `api_surface.rs`, `compile_fail.rs` — the edges.

`support/` holds the shared fixtures: a greeting task/workflow and a small order domain (`shop.rs`) whose tasks count
their side effects, so tests can assert how many times something really ran.

## Suggested order to make it pass
1. `api_surface`: the types exist and are exported from `jackdaw::prelude`.
2. `quickstart` (a YAML workflow bound to `Workflow<In, Out>` first, then a Rust workflow), then `builder` and `errors`.
3. `tasks` — the `#[task]` macros, `run_task`, retry/timeout/fault mapping.
4. `rust_workflows` — `#[workflow]` with plain `ctx.call`, then `for_each`, `select!`, signals/queries.
5. `replay` — the journal; start with `resume_continues_where_the_process_stopped`.
6. `nesting`, `catalogs`, `events`, and finally the lint cases in `compile_fail`.

## API these tests assume (beyond what API.md spells out)
The tests are the specification; this list is the checklist of signatures they pin down.

**Engine and builder**
- `Engine::builder()` -> `EngineBuilder`: `.store(Redb)`, `.task(Marker)`, `.tasks(Arc<T> | T)` (for `#[tasks] impl T`),
  `.workflow::<W>()` (also registers the child workflows `W` runs), `.catalog::<C>()` (for a `#[catalog] trait C`),
  `.event_buffer_size(usize)`, `.build() -> Result<Engine, Error>`. Registration mistakes are `Error::Invalid` at `build()`:
  a task shadowing a built-in call type, duplicate task names, duplicate workflow identities, a workflow calling an
  unregistered task, a catalog entry that is missing or contradicts its schema.
- `Redb::open(path) -> Result<Redb, Error>`; a store implements both persistence and cache.
- `engine.execute(runnable, input) -> Result<ExecutionHandle<Out>, Error>` where `runnable` is a workflow's run marker
  (`HelloWorkflow::run`, the same way signals and queries are `Approval::approve`) or `&Workflow<In, Out>`, and `input: In`.
  Child workflows are `ctx.run(Fulfil::run, &input)`. (A struct with fields cannot be a value, so the unit-struct form
  `execute(HelloWorkflow, ..)` is not used.) Unknown calls in a YAML workflow fail here with
  `Error::Invalid`, before any task runs.
- `engine.resume::<W>(&id) -> Result<ExecutionHandle<W::Out>, Error>`; resuming under a different workflow version is `Error::Invalid`.
- `engine.run_task(marker, input, TaskOptions) -> Result<Out, Error>`: one task through the full retry/timeout/fault path.
- `engine.status(&id) -> Result<Status, Error>` with `Status::{Running, Completed, Faulted, Cancelled, ..}`.
- `engine.history(&id) -> Result<Vec<Event>, Error>`; `engine.replay_all() -> Result<Vec<Divergence>, Error>` where
  `Divergence { instance_id, .. }: Display`.
- Dropping an `Engine` abandons running instances without writing a terminal event (this is how the tests simulate a crash).

**Handles and events**
- `ExecutionHandle<Out>`: `.id() -> &str`, `.events() -> EventStream` (callable repeatedly), `.cancel().await` (`&self`),
  `.signal(Marker, arg)`, `.query(Marker)`, `.send_event(name, payload)`, and `IntoFuture<Output = Result<Out, Error>>`.
- `EventStream::next(&mut self) -> Option<Event>` (an `async fn`; `EventStream: Send + 'static`); it ends after the terminal event.
- `Event` keeps today's variants and fields (`TaskCompleted { task_name, instance_id, .. }` and so on) and gains
  `.name() -> &'static str` and `.replayed() -> bool`. Events are emitted after they are persisted.

**Tasks and workflows**
- `#[task]` (also `name = "..."`, `cached`, `blocking`) on a free fn generates a CamelCase marker (`greet` -> `Greet`); on a
  method of a `#[tasks] impl` it generates an associated marker (`Shop::charge`). A task takes its input by value, with an
  optional `TaskContext`, and returns `Result<Out, TaskError>`. Tasks are not cached unless `cached`.
- `TaskContext`: `.instance_id()`, `.task_name()`, `.attempt()` (1-based), `.idempotency_key()`.
- `TaskError`: `::retryable(e)`, `::non_retryable(msg)`, `::fault(type_uri).status(u16).detail(str)`.
- `TaskOptions::new().retry(Retry).timeout(Duration)`; `Retry::constant(Duration).max_attempts(n)`, `Retry::exponential(n)`.
- `#[workflow(namespace, name, version [, replay = "lenient"])]` on a struct plus `#[workflow_methods]` on its `impl`, with
  `#[run]`, optional `#[init]`, `#[signal]`, `#[query]`. `#[run]` takes `(ctx: &mut WorkflowContext<Self>, input: In)` and
  returns `WorkflowResult<Out>`. `StepError` and `Fault` convert into `WorkflowError` (so `?` and `.into()` work).
- `WorkflowContext` (methods take `&self` so futures combinators work): `.call(marker, input)` (input by value or
  reference) returning a step that takes `.retry(..)`/`.timeout(..)` before `.await`, `.run(WorkflowMarker, input)`,
  `.run_definition(&Workflow<In, Out>, &input)`, `.for_each(&items, marker)` with `.concurrency(n)`, `.sleep(Duration)`,
  `.wait_condition(|state| bool)`, `.wait_for_event::<T>(name)`, `.now()`, `.random()`, `.uuid()`, `.patched(id) -> bool`.
- `jackdaw::select! { x = step => .., () = ctx.sleep(d) => .. }`; the standard `futures::try_join!`/`join_all` are allowed.
- `Workflow::<In, Out>::from_yaml(&str)` (no default for `In`/`Out`; there is no untyped workflow type), errors are `Error::Invalid`.
- `#[catalog(source = "file://...")] trait C { #[task(name, version)] async fn f(..) -> ..; #[workflow(name, version)] async fn g(..) -> ..; }`
  generates markers `C::f` / `C::g`.

**Errors**
- `Error` is `#[non_exhaustive]` with `Faulted(Fault)`, `Cancelled`, `Timeout { .. }`, `Invalid(_)`, `NonDeterminism(_)`,
  and implements `std::error::Error + Send + Sync + 'static`. `Fault { kind, status, title, detail, instance }`.
- `jackdaw::error_types::{COMMUNICATION, RUNTIME, TIMEOUT, VALIDATION, ..}` are the spec's error type URIs.

**Prelude and helpers**
- `jackdaw::prelude::*` exports everything above, the `JsonSchema` derive (schemars), and a duration extension trait
  implemented for all integer types: `10.millis()`, `30.secs()`, `24.hours()`.
- `jackdaw::testing::TestEngine` (+ `TestEngineBuilder`, same builder methods as `EngineBuilder`): derefs to `Engine`,
  in-memory store, `.clock().elapsed()`, and **self-replay**: every workflow is replayed from its own history after
  it runs, and a divergence is returned as `Error::NonDeterminism`. It relies on tokio's paused clock, so its tests
  use `#[tokio::test(start_paused = true)]`.

## Not covered yet
Listeners (HTTP/gRPC), `#[update]` handlers, `engine.execute_with_id` idempotency, event/cache provider failures,
Postgres/SQLite stores, and the migration of the old `DurableEngine` names. Add them as the API for them is settled.
