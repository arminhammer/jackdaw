FROM jackdaw:latest

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

WORKDIR /app

# Copy Python package files
COPY calculator ./calculator
COPY pyproject.toml ./
COPY spec.proto ./
COPY calculator-api.sw.yaml ./

# Install Python dependencies using uv
RUN uv pip install --system -e .

# Expose gRPC port
EXPOSE 50051

# Note: Base image sets ENTRYPOINT to jackdaw, so we just pass the args
CMD ["run", "calculator-api.sw.yaml", "--debug"]
