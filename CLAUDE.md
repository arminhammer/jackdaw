# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

All build tasks are managed via `just` (see `justfile`).

```bash
just build              # debug build
just build-release      # optimized release build
just build-static       # static musl binary for Linux x86_64 (requires cargo-zigbuild + zig)

just lint               # cargo clippy -D warnings
just fmt                # cargo fmt
just check              # cargo check (fast, no codegen)

just test               # unit tests only (cargo test --lib)
just test-ctk           # CTK conformance suite (cargo test --test ctk_conformance)
just test-examples      # example workflow tests
just test-all           # all tests
just test-listeners     # HTTP/gRPC listener integration tests

# Single test by name
cargo test --lib <test_name>
cargo test --test ctk_conformance <test_name>

# Python package (requires maturin)
just python-develop     # editable install: maturin develop --features python
just python-build       # build wheel: maturin build --release --features python
```

The CI pipeline itself runs as a jackdaw workflow: `just ci` (uses `.ci/ci.sw.yaml`). Use `just ci-reset` to clear persistence/cache DBs before a clean CI run.

## Architecture

### Execution model

`DurableEngine` is the core type (`src/durableengine.rs`). It accepts a `WorkflowDefinition` (parsed YAML) and a JSON input blob, builds a `petgraph::DiGraph` from the workflow tasks, then executes nodes in topological order. Each task execution emits `WorkflowEvent` variants to an async channel; the caller receives an `ExecutionHandle` to stream events or await final output.

Persistence and cache are injected via the `DurableEngineBuilder` (`src/builder.rs`). Defaults are in-memory; SQLite, Redb, and PostgreSQL backends are available in `src/providers/`.

### Task execution pipeline

1. `DurableEngine::execute_task` dispatches to one of the typed task handlers in `src/durableengine/tasks/` (call, run, set, fork, for_loop, switch, try_catch, raise, emit, wait).
2. "run" tasks (`src/providers/executors/`) delegate to `PythonExecutor`, `TypeScriptExecutor`, `RestExecutor`, or `OpenApiExecutor`.
3. Each task's output passes through JQ expression evaluation (`src/expressions.rs`) for `.output.as` transforms before being written back to context.
4. Caching is keyed on SHA-256(task_name + serialized_inputs). A cache hit skips re-execution entirely.

### Key traits

| Trait | File | Purpose |
|---|---|---|
| `Executor` | `src/executor.rs` | Implement to add a new task runtime |
| `PersistenceProvider` | `src/persistence.rs` | Store/retrieve workflow events |
| `CacheProvider` | `src/cache.rs` | Get/set cached task results |
| `Listener` | `src/listeners/` | Add new event source types |

### Python package (`jackdaw/`)

The `python` feature (`Cargo.toml`) activates PyO3 bindings in `src/python.rs`, which compiles to `jackdaw/_jackdaw.so` (module entry point: `#[pymodule] fn _jackdaw`). The `pyo3-asyncio-0-21` crate bridges Tokio futures into Python's asyncio.

The `jackdaw/` directory is the Python package source (maturin mixed layout). `jackdaw/__init__.py` re-exports the compiled extension types and the `WorkflowRunner` / `run` / `run_async` helpers from `jackdaw/runner.py`. `runner.py` accepts `serverlessworkflow.sdk.Workflow` objects directly, calls `workflow.to_yaml()`, and passes the result to the compiled engine.

Build with maturin (`just python-build` / `just python-develop`). Tests live in `jackdaw/tests/` and run with `pytest`.

### Workflow YAML parsing

Workflows are parsed by `serverless_workflow_core` (git dependency, Serverless Workflow SDK for Rust). The parsed `WorkflowDefinition` is what `DurableEngine::execute` accepts. The jackdaw codebase does not own the DSL schema — it owns the execution semantics on top of it.

### Listeners

`src/listeners/http.rs` and `src/listeners/grpc.rs` implement long-running servers that trigger workflow executions on incoming requests. OpenAPI and gRPC listener examples in `examples/` show the full pattern. The listener setup lives in `src/durableengine/listeners.rs`.

### Expression evaluation

`src/expressions.rs` wraps the `jaq-*` family of crates. All `${...}` expressions in workflow YAML run through this module. It applies null-safe preprocessing before evaluation so missing fields don't abort execution.
