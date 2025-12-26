# Python gRPC Listener Example

This example demonstrates how to build a gRPC API service using Jackdaw with Python handlers.

## Overview

The example implements a Calculator gRPC API with three methods:
- `calculator.Calculator/Add` - Add two numbers
- `calculator.Calculator/Multiply` - Multiply two numbers
- `calculator.Calculator/GetPet` - Fetch pet information from petstore API (demonstrates third-party dependencies)

The workflow uses gRPC listeners to receive requests, validates them against a protobuf schema, and routes them to Python handler functions.

## Project Structure

```
python-grpc-listener/
├── Dockerfile                 # Container image definition
├── calculator-api.sw.yaml     # Serverless Workflow definition
├── spec.proto                 # Protobuf service specification
├── pyproject.toml             # Python package configuration (includes requests dep)
├── calculator/                # Python handler package
│   ├── __init__.py
│   ├── types.py              # TypedDict definitions
│   ├── add.py                # Add operation handler
│   ├── multiply.py           # Multiply operation handler
│   └── get_pet.py            # Get pet handler (uses requests library)
└── README.md
```

## How It Works

### 1. gRPC Listeners

The workflow defines three gRPC listener tasks that bind to port `50051`:

```yaml
- handleAddRequests:
    listen:
      to:
        any:
          - with:
              source:
                uri: grpc://0.0.0.0:50051/calculator.Calculator/Add
                schema:
                  format: grpc
                  resource:
                    endpoint: spec.proto
                    name: AddRequest
        until: '${ false }'  # Listen forever
```

### 2. Schema Validation

Each request is validated against the protobuf schema defined in `spec.proto`. The proto file defines the service interface and message types, ensuring type safety at the API boundary.

### 3. Handler Execution

When a request arrives, the listener extracts the request data and passes it to the Python handler:

```yaml
foreach:
  item: event
  do:
    - executeAdd:
        call: python
        with:
          module: calculator.add
          function: handler
          arguments:
            - ${ $event }
```

### 4. Type Safety

Python handlers use `TypedDict` for strong typing that matches the protobuf definitions:

```python
from calculator.types import AddRequest, AddResponse

def handler(request: AddRequest) -> AddResponse:
    result: int = request["a"] + request["b"]
    response: AddResponse = {"result": result}
    return response
```

## Building and Running

### Prerequisites

1. Build the jackdaw base image:
   ```bash
   just docker-build
   ```

### Build the Calculator API Image

From the project root:

```bash
docker build -t calculator-grpc-api -f examples/python-grpc-listener/Dockerfile .
```

### Run the Container

```bash
docker run --rm -p 50051:50051 calculator-grpc-api
```

The gRPC server will start and listen on port `50051`.

## Testing the API

You can test the gRPC API using `grpcurl` (install from https://github.com/fullstorydev/grpcurl):

### Add Two Numbers

```bash
grpcurl -plaintext -d '{"a": 5, "b": 3}' \
  localhost:50051 calculator.Calculator/Add
```

Expected response:
```json
{
  "result": 8
}
```

### Multiply Two Numbers

```bash
grpcurl -plaintext -d '{"a": 7, "b": 6}' \
  localhost:50051 calculator.Calculator/Multiply
```

Expected response:
```json
{
  "result": 42
}
```

### Get Pet Information (Third-Party Dependency Example)

This method demonstrates using third-party Python libraries (`requests`) in handlers:

```bash
grpcurl -plaintext -d '{"pet_id": 1}' \
  localhost:50051 calculator.Calculator/GetPet
```

Expected response (fetched from petstore API):
```json
{
  "id": 1,
  "name": "doggie",
  "status": "available"
}
```

This demonstrates that:
- Third-party dependencies (like `requests`) work correctly in Jackdaw containers
- Handlers can make external HTTP calls
- Protobuf field naming (snake_case in proto, converted to/from Python conventions)

## Development

To work on the handlers locally:

1. Install the package in development mode:
   ```bash
   cd examples/python-grpc-listener
   uv pip install -e .
   ```

2. Run the workflow directly (without Docker):
   ```bash
   jackdaw run calculator-api.sw.yaml --debug
   ```

## Production Considerations

For production deployments:

1. **Remove `--debug` flag** from the Dockerfile CMD
2. **Add health checks** using gRPC health checking protocol
3. **Configure TLS** for secure gRPC communication
4. **Set resource limits** in your container orchestrator
5. **Add monitoring** and distributed tracing
6. **Use connection pooling** for external API calls (like petstore)