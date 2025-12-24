# Dockerfile for Calculator OpenAPI Listener Example
# Uses pre-built jackdaw:latest as base image
# Build jackdaw base image first with: just docker-build

FROM jackdaw:latest

# Set working directory
WORKDIR /app

# Copy Python calculator module
COPY calculator ./calculator
COPY pyproject.toml ./

# Copy OpenAPI spec
COPY openapi.spec.yaml ./

# Copy workflow definition
COPY calculator-api.sw.yaml ./

# Install Python package using uv
RUN uv pip install --system -e .

# Expose HTTP port for the REST API
EXPOSE 8080

# Run the workflow with HTTP listener
# This starts the workflow which listens on http://localhost:8080
# Note: Base image sets ENTRYPOINT to jackdaw, so we just pass the args
CMD ["run", "calculator-api.sw.yaml", "--debug"]
