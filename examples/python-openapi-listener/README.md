# Python OpenAPI Listener Example

This example demonstrates how to package a Python module with Jackdaw workflow into a self-contained Docker container that runs as a REST API server.

## Overview

The example implements a Calculator REST API with three endpoints:
- `POST /api/v1/add` - Add two numbers
- `POST /api/v1/multiply` - Multiply two numbers
- `GET /api/v1/pet/{petId}` - Fetch pet information from petstore API (demonstrates third-party dependencies)

The workflow uses HTTP listeners to receive requests, validates them against an OpenAPI schema, and routes them to Python handler functions.

## Project Structure

```
python-openapi-listener/
├── Dockerfile                 # Container image definition
├── calculator-api.sw.yaml     # Serverless Workflow definition
├── openapi.spec.yaml          # OpenAPI 3.0 specification
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

### 1. Workflow Listeners

The workflow (`calculator-api.sw.yaml`) sets up HTTP listeners for each endpoint:

```yaml
- handleAddRequests:
    listen:
      to:
        any:
          - with:
              source:
                uri: http://localhost:8080/api/v1/add
                schema:
                  format: openapi
                  resource:
                    endpoint: openapi.spec.yaml
                    name: AddRequest
      until: ${ false }  # Listen forever
```

### 2. OpenAPI Schema Validation

Incoming requests are validated against the OpenAPI schema defined in `openapi.spec.yaml`:

```yaml
AddRequest:
  type: object
  required:
    - a
    - b
  properties:
    a:
      type: integer
      format: int32
    b:
      type: integer
      format: int32
```

### 3. Python Handler Execution

Valid requests are passed to Python handler functions:

```python
# calculator/add.py
from calculator.types import AddRequest, AddResponse

def handler(request: AddRequest) -> AddResponse:
    result: int = request["a"] + request["b"]
    response: AddResponse = {"result": result}
    return response
```

### 4. Type Safety

The handlers use Python `TypedDict` for strong typing, matching the OpenAPI schema:

```python
# calculator/types.py
from typing import TypedDict

class AddRequest(TypedDict):
    a: int
    b: int

class AddResponse(TypedDict):
    result: int
```

## Building and Running

### Prerequisites

1. Build the base Jackdaw Docker image:
   ```bash
   just docker-build
   ```

### Build the Example Container

From the repository root:

```bash
docker build -t calculator-api -f examples/python-openapi-listener/Dockerfile .
```

### Run the Container

```bash
docker run -p 8080:8080 calculator-api
```

The API server will start and listen on http://localhost:8080

## Testing the API

### Add Two Numbers

```bash
curl -X POST http://localhost:8080/api/v1/add \
  -H "Content-Type: application/json" \
  -d '{"a": 15, "b": 27}'
```

Expected response:
```json
{
  "result": 42
}
```

### Multiply Two Numbers

```bash
curl -X POST http://localhost:8080/api/v1/multiply \
  -H "Content-Type: application/json" \
  -d '{"a": 7, "b": 6}'
```

Expected response:
```json
{
  "result": 42
}
```

### Get Pet Information (Third-Party Dependency Example)

This endpoint demonstrates using third-party Python libraries (`requests`) in handlers:

```bash
curl http://localhost:8080/api/v1/pet/1
```

Expected response (fetched from petstore API):
```json
{
  "id": 1,
  "name": "doggie",
  "status": "available",
  "category": {
    "id": 1,
    "name": "Dogs"
  },
  "photoUrls": [
    "string"
  ],
  "tags": [
    {
      "id": 0,
      "name": "string"
    }
  ]
}
```

## Development

### Local Development Without Docker

1. Install dependencies:
   ```bash
   cd examples/python-openapi-listener
   uv pip install -e .
   ```

2. Run the workflow directly:
   ```bash
   jackdaw run calculator-api.sw.yaml --debug
   ```

3. Test with curl as shown above

### Adding New Operations

1. Add the operation to `openapi.spec.yaml`
2. Create request/response TypedDicts in `calculator/types.py`
3. Implement the handler in a new file (e.g., `calculator/divide.py`)
4. Add a listener task in `calculator-api.sw.yaml`

## Production Considerations

For production use, consider:

- Using persistent storage providers (PostgreSQL, SQLite) instead of in-memory
- Enabling cache providers for performance
- Adding authentication/authorization to the OpenAPI spec
- Implementing rate limiting and error handling
- Setting up health check endpoints
- Using environment variables for configuration
