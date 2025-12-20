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
    cargo build --release

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

test-examples:
    cargo test --test example_tests

test-validate:
    cargo test --test validate_tests

# Run all tests (unit + integration)
test-all:
    cargo test

# Run Docker integration tests (requires Docker image to be built)
test-docker:
    cargo test --test docker_integration_tests

# Build Docker image and run integration tests
test-docker-full: docker-build test-docker

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
# Docker Builds (Linux cross-compilation)
# ============================================================================

# Build Docker image for current platform
docker-build:
    docker buildx build --load -t jackdaw:latest .

# Build Docker image for multiple platforms (amd64 and arm64)
docker-build-multiplatform:
    docker buildx build --platform linux/amd64,linux/arm64 -t jackdaw:latest .

# Build and push Docker image for multiple platforms
docker-build-push:
    docker buildx build --platform linux/amd64,linux/arm64 -t jackdaw:latest --push .

# Build Linux x86_64 binary using Docker (extracts from builder stage)
docker-build-linux-amd64:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building Linux x86_64 binary in Docker..."
    docker buildx build \
        --platform linux/amd64 \
        --target builder \
        --load \
        -t jackdaw-builder:linux-amd64 .
    echo "Extracting binary..."
    mkdir -p dist
    docker create --name jackdaw-extract jackdaw-builder:linux-amd64
    docker cp jackdaw-extract:/build/target/release/jackdaw ./dist/jackdaw-linux-amd64
    docker rm jackdaw-extract
    echo "✓ Linux x86_64 binary ready: ./dist/jackdaw-linux-amd64"
    ls -lh ./dist/jackdaw-linux-amd64
    file ./dist/jackdaw-linux-amd64

# Build Linux ARM64 binary using Docker (extracts from builder stage)
docker-build-linux-arm64:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building Linux ARM64 binary in Docker..."
    docker buildx build \
        --platform linux/arm64 \
        --target builder \
        --load \
        -t jackdaw-builder:linux-arm64 .
    echo "Extracting binary..."
    mkdir -p dist
    docker create --name jackdaw-extract jackdaw-builder:linux-arm64
    docker cp jackdaw-extract:/build/target/release/jackdaw ./dist/jackdaw-linux-arm64
    docker rm jackdaw-extract
    echo "✓ Linux ARM64 binary ready: ./dist/jackdaw-linux-arm64"
    ls -lh ./dist/jackdaw-linux-arm64
    file ./dist/jackdaw-linux-arm64

# Build both Linux binaries (amd64 and arm64)
docker-build-linux: docker-build-linux-amd64 docker-build-linux-arm64
    @echo ""
    @echo "✓ All Linux binaries built:"
    @ls -lh ./dist/jackdaw-linux-*

# Build all release binaries (Linux via Docker, macOS native)
docker-build-all: docker-build-linux build-macos-universal
    @echo ""
    @echo "✓ All release binaries ready:"
    @ls -lh ./dist/jackdaw-*
    @ls -lh ./target/universal-apple-darwin/release/jackdaw