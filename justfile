# Setup Python library symlink for pyo3 (required on immutable systems)
setup-python:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Setting up Python library symlink for pyo3..."
    mkdir -p ~/lib
    if [ ! -f ~/lib/libpython3.13.so ]; then
        if [ -f /usr/lib64/libpython3.13.so.1.0 ]; then
            ln -sf /usr/lib64/libpython3.13.so.1.0 ~/lib/libpython3.13.so
            echo "✓ Created symlink: ~/lib/libpython3.13.so -> /usr/lib64/libpython3.13.so.1.0"
        else
            echo "✗ Error: /usr/lib64/libpython3.13.so.1.0 not found"
            echo "  Please install Python 3.13 or adjust the path in justfile"
            exit 1
        fi
    else
        echo "✓ Symlink already exists: ~/lib/libpython3.13.so"
    fi
    echo "✓ Python setup complete"

# Build the project
build:
    cargo build --release

# Build in development mode
build-dev:
    cargo build

# Run all unit tests
test:
    cargo test --lib

# Run all CTK integration tests (consolidated report)
test-ctk:
    cargo test --test ctk_conformance

# Run CTK tests and show only summaries (ignore failures)
test-ctk-summary:
    -cargo test --test ctk_conformance 2>&1 | grep -E "(Running CTK|Feature:|Summary)"

# Run CTK tests with verbose output
test-ctk-verbose:
    RUST_BACKTRACE=1 cargo test --test ctk_conformance

# View CTK conformance status
ctk-status:
    cat CTK_STATUS.md

# List all CTK feature worlds
test-ctk-list:
    @echo "CTK feature worlds:"
    @ls tests/worlds/*.rs | grep -v mod.rs | xargs -n1 basename -s .rs | sed 's/^/  - /'

# Run all tests (unit + integration)
test-all:
    cargo test

# Clean build artifacts
clean:
    cargo clean

# Format code
fmt:
    cargo fmt

# Run clippy linter
lint:
    cargo clippy -- -D warnings

# Check code without building
check:
    cargo check

# Run listener tests (gRPC and HTTP/OpenAPI)
test-listeners:
    cargo test --test listener_tests --no-fail-fast

# Run full CI pipeline locally
ci:
    cargo build --release
    ./target/release/mooose run .ci/ci.sw.yaml \
        --durable-db .ci/ci-persistence.db \
        --cache-db .ci/ci-cache.db \
        --verbose

# Run CI and force re-execution (skip cache)
ci-clean:
    cargo build --release
    ./target/release/mooose run .ci/ci.sw.yaml \
        --durable-db .ci/ci-persistence.db \
        --cache-db .ci/ci-cache.db \
        --no-cache \
        --verbose

# Clean CI databases
ci-reset:
    rm -f .ci/ci-persistence.db .ci/ci-cache.db