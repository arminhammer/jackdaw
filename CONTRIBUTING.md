# Contributing to jackdaw

## The workspace

| Crate | Directory | What it is |
|---|---|---|
| `jackdaw-wf` (imported as `jackdaw`) | `jackdaw/` | The library: the engine, the API, the stores, the YAML interpreter. |
| `jackdaw-wf-macros` | `jackdaw-macros/` | `#[task]`, `#[tasks]`, `#[workflow]`, `#[workflow_methods]`, `#[catalog]`. |
| `jackdaw-cli` | `jackdaw-cli/` | The `jackdaw` binary, built only on the library's public API. |

The development toolchain is pinned in `rust-toolchain.toml`; the oldest supported one is the workspace's
`rust-version` (1.94), checked in CI. Run `just init` once for the cross-compilation tools and `just submodules` for
the Open Workflow Specification and conformance tests.

## Tests

- `just test-lib`: the library's executable specification, Gherkin features in `jackdaw/tests/lib` run with
  cucumber-rs. Scenarios tagged `@todo` describe behavior that is not built yet and are skipped; turning one into
  `@implemented` (with step definitions in `jackdaw/tests/lib_steps`) is how features land. `@docker` scenarios need
  Docker and `@postgres` ones `DATABASE_URL`.
  - `JACKDAW_TEST_STORE=sqlite` or `postgres` (with `DATABASE_URL` pointing at a server where the user may create
    databases) runs the durable scenarios on that store instead of Redb.
  - `JACKDAW_SCENARIO_CONCURRENCY` (default 8) and `JACKDAW_SCENARIO_TIMEOUT` (seconds, default 600). A scenario that
    overruns ends the run and is named; `JACKDAW_HANG_DEBUGGER=gdb` (or `lldb`) also prints every thread's stack.
- `cargo test -p jackdaw-wf --test rust`: the Rust API tests, including compile-fail cases (`tests/rust/ui`; refresh
  their expected output with `TRYBUILD=overwrite`).
- `just test-all`: everything, including the YAML conformance and listener suites.
- `cargo hack clippy -p jackdaw-wf --each-feature --no-dev-deps`: each Cargo feature on its own. The tests always build
  with every feature, so this is where a missing `#[cfg]` shows up.

`cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo fmt --all` must be clean; the workspace
denies wildcard matches on enums, indexing that can panic, and a few other lints (see the root `Cargo.toml`).

## Vendored code

`jackdaw/src/dsl` is the Open Workflow Specification Rust SDK's model, copied verbatim. Do not edit it: change it upstream (or in
the fork that `just update-dsl` pulls from) and run `just update-dsl [repo] [ref]`, which records the commit in
`jackdaw/src/dsl/UPSTREAM`.

## Stability

Jackdaw is before 1.0, so a minor version may break the API; the changelog marks every breaking change.

- **Meant to be stable** (changed only with a minor version and a changelog entry): `jackdaw::api`, `jackdaw::prelude`,
  `jackdaw::testing`, the macros' attributes, the store traits (`PersistenceProvider`, `CacheProvider`, `Store`), and
  the journal format, which a new release keeps reading (see the durability chapter of the guide).
- **Follows the specification, not jackdaw's own pace:** `jackdaw::dsl` mirrors the Open Workflow DSL and moves
  with it.
- **Not part of the API:** `jackdaw::__private` (what the macros expand to) and anything `#[doc(hidden)]`.

Public enums and error types are `#[non_exhaustive]` so they can grow without a breaking release.

## Releasing

1. Bump `version` in `[workspace.package]` and the two `=x.y.z` pins in `[workspace.dependencies]` (root
   `Cargo.toml`), and move the changelog's Unreleased section under the new version.
2. `just publish-dry-run` must pass.
3. Merge to `main`. The release workflow builds the binaries, the Docker image and the GitHub release when the version
   changes, then publishes the crates if the repository variable `PUBLISH_CRATES` is `true` (with the
   `CARGO_REGISTRY_TOKEN` secret). Until that is set, publish by hand with `just publish-crate`, after the GitHub
   release exists, so `cargo binstall jackdaw-cli` finds its binary.
