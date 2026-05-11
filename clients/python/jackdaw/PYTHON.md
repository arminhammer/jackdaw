# Python Bindings for Jackdaw

Jackdaw provides optional Python bindings using PyO3, allowing you to execute Serverless Workflows from Python applications.

## Requirements

- Python 3.8+
- Rust toolchain (for building from source)
- [maturin](https://github.com/PyO3/maturin) - Python/Rust build tool

## Installation

### From Source

1. Install maturin:
```bash
pip install maturin
```

2. Build and install the Python module with the python feature enabled:
```bash
maturin develop --features python
```

For production builds:
```bash
maturin build --release --features python
pip install target/wheels/jackdaw-*.whl
```

## Usage

### Basic Workflow Execution

```python
import asyncio
import jackdaw

async def main():
    # Create engine using builder
    builder = jackdaw.DurableEngineBuilder()
    engine = builder.build()

    # Define workflow as YAML string
    workflow_yaml = """
    document:
      dsl: '1.0.2'
      namespace: examples
      name: hello-world
      version: 1.0.0
    do:
      - greet:
          set:
            message: "Hello, World!"
    """

    # Execute workflow
    handle = await engine.execute(workflow_yaml, {"user": "Alice"})

    # Wait for completion
    result = await handle.wait_for_completion(30.0)
    print(result)

asyncio.run(main())
```

### Custom Database Configuration

```python
import asyncio
import jackdaw

async def main():
    builder = jackdaw.DurableEngineBuilder()

    # Configure custom databases
    builder.with_state_database("sqlite:///workflow_state.db")
    builder.with_result_database("sqlite:///workflow_results.db")

    engine = builder.build()

    # Use the engine...

asyncio.run(main())
```

### Checking Workflow Status

```python
import asyncio
import jackdaw

async def main():
    builder = jackdaw.DurableEngineBuilder()
    engine = builder.build()

    handle = await engine.execute(workflow_yaml, input_data)

    print(f"Instance ID: {handle.instance_id()}")
    print(f"Completed: {handle.is_completed()}")

    # Poll for completion
    while not handle.is_completed():
        await asyncio.sleep(0.1)

    result = await handle.wait_for_completion(30.0)
    print(result)

asyncio.run(main())
```

## API Reference

### DurableEngineBuilder

Builder for creating a `DurableEngine` instance.

**Methods:**
- `__init__()` - Create a new builder
- `with_state_database(db_url: str)` - Configure state database URL
- `with_result_database(db_url: str)` - Configure result database URL
- `build() -> DurableEngine` - Build the engine

### DurableEngine

The main workflow execution engine.

**Methods:**
- `builder() -> DurableEngineBuilder` - Create a new builder (static method)
- `execute(workflow_yaml: str, input: dict) -> ExecutionHandle` - Execute a workflow (async)

### ExecutionHandle

Handle for monitoring and controlling workflow execution.

**Methods:**
- `instance_id() -> str` - Get the workflow instance ID
- `is_completed() -> bool` - Check if workflow has completed
- `wait_for_completion(timeout_secs: float) -> dict` - Wait for workflow to complete (async)

## Examples

See the `examples/python/` directory for complete examples:

- `simple_workflow.py` - Basic workflow execution
- `event_streaming.py` - Workflow with custom database configuration

Run examples:
```bash
python examples/python/simple_workflow.py
python examples/python/event_streaming.py
```

## Building for Distribution

To build wheels for distribution:

```bash
# Build for current platform
maturin build --release --features python

# Or use just command
just python-build
```

Wheels will be placed in `target/wheels/`.

## Publishing to PyPI

### Prerequisites

1. Create an account at [PyPI](https://pypi.org)
2. Get your API token from [PyPI account settings](https://pypi.org/manage/account/)
3. Configure maturin with your token:
```bash
# Set environment variable
export MATURIN_PYPI_TOKEN=<your-token>

# Or configure in ~/.pypirc
[pypi]
username = __token__
password = <your-token>
```

### Manual Publishing

```bash
# Build wheels
just python-build

# Publish to PyPI
just python-publish

# Or publish to TestPyPI for testing
just python-publish-test
```

### Automated Publishing with GitHub Actions

For multi-platform wheels (Linux, macOS, Windows), use GitHub Actions.

Create `.github/workflows/publish-python.yml`:

```yaml
name: Publish Python Package

on:
  release:
    types: [published]

permissions:
  contents: read

jobs:
  build-wheels:
    name: Build wheels on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]

    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-python@v5
        with:
          python-version: '3.12'

      - name: Install maturin
        run: pip install maturin

      - name: Build wheels
        run: maturin build --release --features python

      - name: Upload wheels
        uses: actions/upload-artifact@v4
        with:
          name: wheels-${{ matrix.os }}
          path: target/wheels/*.whl

  publish:
    name: Publish to PyPI
    needs: build-wheels
    runs-on: ubuntu-latest
    environment: release

    steps:
      - uses: actions/download-artifact@v4
        with:
          pattern: wheels-*
          merge-multiple: true
          path: wheels

      - name: Publish to PyPI
        env:
          MATURIN_PYPI_TOKEN: ${{ secrets.PYPI_TOKEN }}
        run: |
          pip install maturin
          maturin upload wheels/*.whl
```

Setup:
1. Add your PyPI token to GitHub repository secrets as `PYPI_TOKEN`
2. Create a GitHub release - the workflow will automatically build and publish

## Notes

- The Python bindings are completely optional and do not affect the core Rust library or CLI binary
- The CLI binary is built without the `python` feature and remains a static musl binary
- Only enable the `python` feature when building Python wheels with maturin
- Workflow definitions are passed as YAML strings (programmatic workflow construction from Python is not yet supported)

## Publishing Both Rust Crate and Python Package

For a complete release:

```bash
# Publish Rust crate to crates.io
just publish-crate

# Publish Python package to PyPI
just publish-python

# Or do both at once
just publish-all
```
