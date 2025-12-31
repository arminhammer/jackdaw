# Multi-platform Dockerfile for jackdaw
# Supports: linux/amd64, linux/arm64
# Uses static compilation with musl for minimal binary size
# Final image includes Python 3 and Node.js for external script execution
# Optimized for dependency caching
#
# Build with:
#   docker buildx build --platform linux/amd64,linux/arm64 -t jackdaw:latest .
#   docker buildx build --platform linux/amd64,linux/arm64 -t jackdaw:latest --push .

# =============================================================================
# Builder Stage - Static compilation with Alpine + musl
# =============================================================================
FROM rust:alpine AS builder

# Set platform args (provided by buildx)
ARG TARGETPLATFORM
ARG TARGETARCH

# Install build dependencies for static compilation
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig \
    git \
    ca-certificates

# Install Rust nightly for edition 2024 support
RUN rustup toolchain install nightly && \
    rustup default nightly && \
    rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

# Set working directory
WORKDIR /build

# =============================================================================
# DEPENDENCY CACHING LAYER
# Copy only Cargo files and submodules first for maximum cache efficiency
# This layer will be cached as long as dependencies don't change
# =============================================================================
COPY Cargo.toml Cargo.lock ./
COPY submodules ./submodules

# Create dummy source files to build dependencies
# This allows cargo to compile all dependencies without the actual source
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn lib() {}" > src/lib.rs

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
    cargo build --release --target $RUST_TARGET 2>&1 | head -100 || true

# Clean up the dummy build artifacts but keep the dependency cache
RUN rm -rf src

# =============================================================================
# ACTUAL BUILD LAYER
# Copy actual source code and build the real binary
# This layer only rebuilds if source code changes
# =============================================================================
COPY src ./src
COPY tests ./tests

# Build the actual jackdaw binary with static linking
# The release profile in Cargo.toml uses:
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
    cargo build --release --target $RUST_TARGET --bin jackdaw && \
    cp target/$RUST_TARGET/release/jackdaw /build/jackdaw

# Additional stripping to ensure minimal size (if not already stripped)
RUN strip /build/jackdaw || true

# Verify the binary is static
RUN ldd /build/jackdaw || echo "Static binary (no dependencies)"

# =============================================================================
# Final Stage - Minimal runtime image with Python + Node.js
# =============================================================================
FROM alpine:latest AS final

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
COPY --from=builder /build/jackdaw /usr/local/bin/jackdaw

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
LABEL org.opencontainers.image.description="A durable, cached, graph-based execution engine for Serverless Workflow"
LABEL org.opencontainers.image.authors="Armin Graf"
LABEL org.opencontainers.image.source="https://github.com/arminhammer/jackdaw"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL org.opencontainers.image.version="0.1.0"
LABEL org.opencontainers.image.vendor="Jackdaw Project"
