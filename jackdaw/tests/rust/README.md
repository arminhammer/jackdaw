# Library API tests (`tests/rust`)

The executable specification of the Rust library API. The API itself is documented in rustdoc (`cargo doc -p jackdaw
--open`; start at the `jackdaw::guide` chapters). These tests pin down the behaviour behind that documentation.

```
cargo test -p jackdaw --test rust                 # everything
cargo test -p jackdaw --test rust tasks::         # one module
TRYBUILD=overwrite cargo test -p jackdaw --test rust compile_fail   # write/refresh ui/*.stderr
```

## Reading order
1. `quickstart.rs`: the two ways to run something, a YAML workflow bound to types and a workflow written in Rust.
2. `tasks.rs`: `#[task]`, errors, retries, timeouts, caching.
3. `rust_workflows.rs`: `#[workflow]`: control flow, parallelism, signals, queries, waiting.
4. `nesting.rs`, `catalogs.rs`: reuse, with child workflows (Rust runs Rust or YAML) and `#[catalog]`.
5. `replay.rs`: durability, evolving code, `NonDeterminism`.
6. `events.rs`, `errors.rs`, `builder.rs`, `api_surface.rs`, `compile_fail.rs`: the edges.

`support/` holds the shared fixtures: a greeting task/workflow and a small order domain (`shop.rs`) whose tasks count
their side effects, so tests can assert how many times something really ran.

## Conventions
- A test that crashes an engine does it by dropping the `Engine`, which abandons running instances without writing a
  terminal event. A new engine over the same store then `resume`s them.
- Tests that use time run under `#[tokio::test(start_paused = true)]` through `jackdaw::testing::TestEngine`, which also
  replays every run from its own journal and fails on a divergence.
- `compile_fail.rs` uses `trybuild`; the expected compiler output is in `ui/*.stderr` and depends on the rustc version.

## Not covered yet
Listeners (HTTP/gRPC), `#[update]` handlers, `engine.execute_with_id` idempotency, event/cache provider failures,
Postgres/SQLite stores, and the migration of the old `DurableEngine` names.
