# JavaScript (ES2024) gRPC Listener Example

This example demonstrates how to build a gRPC API service using Jackdaw with JavaScript ES2024 handlers.

## Overview

The example implements a Calculator gRPC API with three methods:
- `calculator.Calculator/Add` - Add two numbers
- `calculator.Calculator/Multiply` - Multiply two numbers
- `calculator.Calculator/GetPet` - Fetch pet information from petstore API (demonstrates third-party npm packages)

The workflow uses gRPC listeners to receive requests, validates them against a protobuf schema, and routes them to JavaScript handler functions.

## Project Structure

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
        until: '\${ false }'  # Listen forever
```

### 2. Schema Validation

Each request is validated against the protobuf schema defined in `spec.proto`. The proto file defines the service interface and message types, ensuring type safety at the API boundary.

### 3. Handler Execution

When a request arrives, the listener extracts the request data and passes it to the JavaScript handler:

```yaml
foreach:
  item: event
  do:
    - executeAdd:
        call: javascript  # Uses Deno runtime for ES2024 JavaScript
        with:
          module: src/add.js
          function: handler
          arguments:
            - \${ \$event }
```

### 4. JavaScript ES2024 Handlers

Handlers are pure JavaScript (ES2024) with JSDoc for documentation:

```javascript
/**
 * @param {Object} request - AddRequest with 'a' and 'b' as int32 values
 * @param {number} request.a - First operand
 * @param {number} request.b - Second operand
 * @returns {Object} AddResponse with 'result' field containing the sum
 */
export function handler(request) {
  const result = request.a + request.b;
  const response = { result };
  return response;
}
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
docker build -t calculator-js-grpc-api -f examples/javascript-grpc-listener/Dockerfile .
```

### Run the Container

```bash
docker run --rm -p 50051:50051 calculator-js-grpc-api
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

This method demonstrates using third-party npm packages (`axios`) in JavaScript handlers:

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
