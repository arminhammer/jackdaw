# Pull and sync git submodules
submodules:
    git submodule update --init --recursive

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
    cargo build

# Build in development mode
build-release:
    RUSTFLAGS="-D warnings" cargo build --release

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

# Generate code coverage report for all tests (unit and integration tests)
coverage:
    cargo llvm-cov \
        --all-features \
        --workspace \
        --html \
        --open

# Generate coverage report without opening browser
coverage-report:
    cargo llvm-cov \
        --all-features \
        --workspace \
        --html

# Clean coverage data
coverage-clean:
    cargo llvm-cov clean

# Clean build artifacts
clean:
    cargo clean

# Format code
fmt:
    cargo fmt

# Run clippy linter
lint:
    cargo clippy -- -D warnings

# Check for potential panics (unwrap, expect, etc.)
lint-panics:
    cargo clippy -- \
        -W clippy::unwrap_used \
        -W clippy::expect_used \
        -W clippy::panic \
        -W clippy::indexing_slicing

# Check code without building
check:
    cargo check

# Run listener tests (gRPC and HTTP/OpenAPI)
test-listeners:
    cargo test --test listener_tests --no-fail-fast

# Run nested workflow tests
test-nested-workflows:
    cargo test --test nested_workflow_tests --no-fail-fast

# Run full CI pipeline locally
ci:
    cargo build
    ./target/debug/jackdaw run .ci/ci.sw.yaml \
        --durable-db .ci/ci-persistence.db \
        --cache-db .ci/ci-cache.db \
        --verbose

# Run CI and force re-execution (skip cache)
ci-clean:
    cargo build
    ./target/debug/jackdaw run .ci/ci.sw.yaml \
        --durable-db .ci/ci-persistence.db \
        --cache-db .ci/ci-cache.db \
        --no-cache \
        --verbose

# Clean CI databases
ci-reset:
    rm -f .ci/ci-persistence.db .ci/ci-cache.db

# ============================================================================
# Release Binary Building
# ============================================================================

# Build optimized release binary for current platform
build-release-optimized:
    cargo build --release

# Build static Linux binary (x86_64, musl)
# Note: Builds V8 from source (takes 30-60 minutes on first build)
build-linux-amd64:
    V8_FROM_SOURCE=1 cargo zigbuild --release --target x86_64-unknown-linux-musl

# Build static Linux binary (ARM64, musl)
# Note: Builds V8 from source (takes 30-60 minutes on first build)
build-linux-arm64:
    V8_FROM_SOURCE=1 cargo zigbuild --release --target aarch64-unknown-linux-musl

# Build macOS binary (Intel)
build-macos-amd64:
    cargo build --release --target x86_64-apple-darwin

# Build macOS binary (Apple Silicon)
build-macos-arm64:
    cargo build --release --target aarch64-apple-darwin

# Build universal macOS binary (combines Intel + Apple Silicon)
build-macos-universal: build-macos-amd64 build-macos-arm64
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target/universal-apple-darwin/release
    lipo -create \
        target/x86_64-apple-darwin/release/jackdaw \
        target/aarch64-apple-darwin/release/jackdaw \
        -output target/universal-apple-darwin/release/jackdaw
    echo "✓ Universal macOS binary created: target/universal-apple-darwin/release/jackdaw"

# Build all release binaries (Linux x86_64, Linux ARM64, macOS universal)
build-all-release: build-linux-amd64 build-linux-arm64 build-macos-universal
    @echo "✓ All release binaries built:"
    @echo "  - target/x86_64-unknown-linux-musl/release/jackdaw"
    @echo "  - target/aarch64-unknown-linux-musl/release/jackdaw"
    @echo "  - target/universal-apple-darwin/release/jackdaw"

# Package release binaries with version tag
package-release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p dist

    # Linux x86_64
    tar -czf dist/jackdaw-{{VERSION}}-linux-amd64.tar.gz \
        -C target/x86_64-unknown-linux-musl/release jackdaw

    # Linux ARM64
    tar -czf dist/jackdaw-{{VERSION}}-linux-arm64.tar.gz \
        -C target/aarch64-unknown-linux-musl/release jackdaw

    # macOS Universal
    tar -czf dist/jackdaw-{{VERSION}}-macos-universal.tar.gz \
        -C target/universal-apple-darwin/release jackdaw

    # Generate checksums
    cd dist
    shasum -a 256 jackdaw-{{VERSION}}-*.tar.gz > jackdaw-{{VERSION}}-checksums.txt
    cd ..

    echo "✓ Release packages created in dist/"
    ls -lh dist/jackdaw-{{VERSION}}-*

# Show binary sizes
show-binary-sizes:
    @echo "Binary sizes:"
    @ls -lh target/x86_64-unknown-linux-musl/release/jackdaw 2>/dev/null || echo "  x86_64-linux-musl: not built"
    @ls -lh target/aarch64-unknown-linux-musl/release/jackdaw 2>/dev/null || echo "  aarch64-linux-musl: not built"
    @ls -lh target/universal-apple-darwin/release/jackdaw 2>/dev/null || echo "  universal-apple-darwin: not built"

# ============================================================================
# Docker Builds
# ============================================================================

# Build static Linux binary in Docker (includes V8 from source)
docker-build-linux-amd64:
    docker build -f Dockerfile.build -t jackdaw-builder:latest .
    docker create --name jackdaw-extract jackdaw-builder:latest
    docker cp jackdaw-extract:/jackdaw ./target/jackdaw-linux-amd64
    docker rm jackdaw-extract
    @echo "✓ Static binary extracted to: ./target/jackdaw-linux-amd64"
    @ls -lh ./target/jackdaw-linux-amd64

# Build and extract binary in one command
docker-build-extract:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building static binary in Docker (this will take 30-60 minutes on first build)..."
    docker build -f Dockerfile.build -t jackdaw-builder:latest .
    echo "Extracting binary..."
    mkdir -p dist
    docker create --name jackdaw-extract jackdaw-builder:latest
    docker cp jackdaw-extract:/jackdaw ./dist/jackdaw-linux-amd64
    docker rm jackdaw-extract
    echo "✓ Static binary ready: ./dist/jackdaw-linux-amd64"
    ls -lh ./dist/jackdaw-linux-amd64
    echo "Verifying static binary..."
    file ./dist/jackdaw-linux-amd64