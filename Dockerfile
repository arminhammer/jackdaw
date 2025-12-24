# Multi-platform Dockerfile for jackdaw
# Supports: linux/amd64, linux/arm64
# Uses Python 3.13 from Ubuntu 25.10 (Oracular Oriole)
# Produces minimal distroless final image with only jackdaw binary + Python shared library
# Uses glibc (not musl) for maximum compatibility
# Optimized for dependency caching
#
# Build with:
#   docker buildx build --platform linux/amd64,linux/arm64 -t jackdaw:latest .
#   docker buildx build --platform linux/amd64,linux/arm64 -t jackdaw:latest --push .

# =============================================================================
# Builder Stage - Uses Ubuntu 25.10 for Python 3.13 support
# =============================================================================
FROM python:3.14-slim AS builder

# Set platform args (provided by buildx)
ARG TARGETPLATFORM
ARG TARGETARCH
ARG PYTHONVERSION=3.13

# Prevent interactive prompts
ENV DEBIAN_FRONTEND=noninteractive

# Install build dependencies and Python 3.13
# Ubuntu 25.10 (Oracular) includes Python 3.13 by default
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    curl \
    ca-certificates \
    python${PYTHONVERSION} \
    python${PYTHONVERSION}-dev \
    libpython${PYTHONVERSION} \
    libpython${PYTHONVERSION}-dev \
    && rm -rf /var/lib/apt/lists/*

# Set Python as default
RUN update-alternatives --install /usr/bin/python3 python3 /usr/bin/python${PYTHONVERSION} 1

# Verify Python installation
RUN python3 --version

# Install Rust nightly for edition 2024 support
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain nightly --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

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

# Build dependencies only (this layer will be cached)
# This downloads and compiles all dependencies including rustyscript with pre-built V8
# The || true allows this to "fail" gracefully as it's building dummy source
RUN cargo build --release 2>&1 | head -100 || true

# Clean up the dummy build artifacts but keep the dependency cache
RUN rm -rf src target/release/jackdaw target/release/deps/jackdaw* \
    target/release/.fingerprint/jackdaw*

# =============================================================================
# ACTUAL BUILD LAYER
# Copy actual source code and build the real binary
# This layer only rebuilds if source code changes
# =============================================================================
COPY src ./src
COPY tests ./tests

# Build the actual jackdaw binary with release optimizations
# The release profile in Cargo.toml uses:
# - opt-level = "z" (optimize for size)
# - lto = true (link-time optimization)
# - codegen-units = 1 (better optimization)
# - strip = true (remove debug symbols)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release

# Additional stripping to ensure minimal size
RUN strip target/release/jackdaw

# =============================================================================
# Final Stage - Distroless Python3 runtime image
# =============================================================================
FROM python:3.14-slim AS final

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

# Copy the jackdaw binary
COPY --from=builder /build/target/release/jackdaw /usr/local/bin/jackdaw

# Set entrypoint to jackdaw binary
ENTRYPOINT ["/usr/local/bin/jackdaw"]
CMD ["--help"]

# OCI metadata labels
LABEL org.opencontainers.image.title="Jackdaw"
LABEL org.opencontainers.image.description="A durable, cached, graph-based execution engine for Serverless Workflows"
LABEL org.opencontainers.image.authors="Armin Graf"
LABEL org.opencontainers.image.source="https://github.com/arminhammer/jackdaw"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL org.opencontainers.image.version="0.1.0"
LABEL org.opencontainers.image.vendor="Jackdaw Project"