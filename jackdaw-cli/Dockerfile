# Multi-platform Dockerfile for jackdaw
# Supports: linux/amd64, linux/arm64
# Uses static compilation with musl for minimal binary size
# Final image includes Python 3 and Node.js for external script execution
# Optimized for dependency caching
#
# Build from the repository root (the build context is the whole workspace):
#   docker buildx build -f jackdaw-cli/Dockerfile --platform linux/amd64,linux/arm64 -t jackdaw:latest .
#   docker buildx build -f jackdaw-cli/Dockerfile --platform linux/amd64,linux/arm64 -t jackdaw:latest --push .
# The ignore rules live in jackdaw-cli/Dockerfile.dockerignore.

# =============================================================================
# Builder Stage - Static compilation with Alpine + musl
# =============================================================================
# The toolchain matches rust-toolchain.toml.
FROM docker.io/library/rust:1.99.0-alpine AS builder

# Set platform args (provided by buildx)
ARG TARGETPLATFORM
ARG TARGETARCH

# Build dependencies for static compilation. musl-dev is the C toolchain for the two C dependencies, ring and the
# bundled SQLite; nothing links OpenSSL.
RUN apk add --no-cache \
    musl-dev \
    ca-certificates

RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

# Set working directory
WORKDIR /build

# =============================================================================
# DEPENDENCY CACHING LAYER
# Copy only the Cargo manifests first for maximum cache efficiency
# This layer will be cached as long as dependencies don't change
# =============================================================================
COPY Cargo.toml Cargo.lock ./
COPY jackdaw/Cargo.toml ./jackdaw/Cargo.toml
COPY jackdaw-macros/Cargo.toml ./jackdaw-macros/Cargo.toml
COPY jackdaw-cli/Cargo.toml ./jackdaw-cli/Cargo.toml

# Create dummy source files to build dependencies
# This allows cargo to compile all dependencies without the actual source.
# The jackdaw manifest declares [[test]] targets, and cargo requires their files to exist.
RUN mkdir -p jackdaw/src jackdaw/tests jackdaw-macros/src jackdaw-cli/src && \
    echo "pub fn lib() {}" > jackdaw/src/lib.rs && \
    touch jackdaw-macros/src/lib.rs && \
    echo "fn main() {}" > jackdaw-cli/src/main.rs && \
    for t in $(sed -n '/^\[\[test\]\]/{n;s/name = "\(.*\)"/\1/p}' jackdaw/Cargo.toml); do \
      echo "fn main() {}" > "jackdaw/tests/$t.rs"; \
    done

# Determine target based on platform
# Use build arg to set the target triple
ARG RUST_TARGET
RUN if [ -z "$RUST_TARGET" ]; then \
      case "$TARGETPLATFORM" in \
        "linux/amd64") export RUST_TARGET=x86_64-unknown-linux-musl ;; \
        "linux/arm64") export RUST_TARGET=aarch64-unknown-linux-musl ;; \
        *) export RUST_TARGET=x86_64-unknown-linux-musl ;; \
      esac; \
    fi && \
    echo "Building for target: $RUST_TARGET" && \
    cargo build --release --target $RUST_TARGET -p jackdaw-cli 2>&1 | head -100 || true

# Clean up the dummy sources but keep the dependency cache.
# The dummy jackdaw/tests files stay: cargo needs them to load the manifest, and `cargo build` never compiles them.
RUN rm -rf jackdaw/src jackdaw-macros/src jackdaw-cli/src

# =============================================================================
# ACTUAL BUILD LAYER
# Copy actual source code and build the real binary
# This layer only rebuilds if source code changes
# =============================================================================
COPY jackdaw/src ./jackdaw/src
COPY jackdaw-macros/src ./jackdaw-macros/src
COPY jackdaw-cli/src ./jackdaw-cli/src

# Build the actual jackdaw binary with static linking
# The release profile in the workspace Cargo.toml uses:
# - opt-level = "z" (optimize for size)
# - lto = true (link-time optimization)
# - codegen-units = 1 (better optimization)
# - strip = true (remove debug symbols)
RUN case "$TARGETPLATFORM" in \
      "linux/amd64") export RUST_TARGET=x86_64-unknown-linux-musl ;; \
      "linux/arm64") export RUST_TARGET=aarch64-unknown-linux-musl ;; \
      *) export RUST_TARGET=x86_64-unknown-linux-musl ;; \
    esac && \
    echo "Building jackdaw for target: $RUST_TARGET" && \
    cargo build --release --target $RUST_TARGET -p jackdaw-cli --bin jackdaw && \
    mkdir -p /out && cp target/$RUST_TARGET/release/jackdaw /out/jackdaw

# Additional stripping to ensure minimal size (if not already stripped)
RUN strip /out/jackdaw || true

# Verify the binary is static
RUN ldd /out/jackdaw || echo "Static binary (no dependencies)"

# =============================================================================
# Final Stage - Minimal runtime image with Python + Node.js
# =============================================================================
FROM docker.io/library/alpine:latest AS final

# Install Python 3 and Node.js for script execution
# These are required by the external executors
RUN apk add --no-cache \
    python3 \
    nodejs \
    npm \
    ca-certificates

# Create a non-root user for running jackdaw
RUN addgroup -S jackdaw && adduser -S jackdaw -G jackdaw

# Copy the statically-linked jackdaw binary
COPY --from=builder /out/jackdaw /usr/local/bin/jackdaw

# Ensure binary is executable
RUN chmod +x /usr/local/bin/jackdaw

# Set working directory
WORKDIR /workspace

# Change ownership to jackdaw user
RUN chown -R jackdaw:jackdaw /workspace

# Switch to non-root user
USER jackdaw

# Verify installations
RUN python3 --version && \
    node --version && \
    jackdaw --version || jackdaw --help

# Set entrypoint to jackdaw binary
ENTRYPOINT ["/usr/local/bin/jackdaw"]
CMD ["--help"]

# OCI metadata labels
LABEL org.opencontainers.image.title="Jackdaw"
LABEL org.opencontainers.image.description="A durable, cached, graph-based execution engine for the Open Workflow DSL (formerly Serverless Workflow)"
LABEL org.opencontainers.image.authors="Armin Graf"
LABEL org.opencontainers.image.source="https://github.com/arminhammer/jackdaw"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL org.opencontainers.image.version="0.2.2"
LABEL org.opencontainers.image.vendor="Jackdaw Project"
