FROM jackdaw:latest

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

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

CMD ["run", "calculator-api.sw.yaml", "--debug"]
