# JavaScript (ES2024) OpenAPI Listener Example

This example demonstrates how to build an HTTP REST API service using Jackdaw with JavaScript ES2024 handlers.

## Overview

The example implements a Calculator REST API with three endpoints:
- `POST /api/v1/add` - Add two numbers
- `POST /api/v1/multiply` - Multiply two numbers
- `GET /api/v1/pet/{petId}` - Fetch pet information from petstore API (demonstrates third-party npm packages)

The workflow uses HTTP listeners to receive requests, validates them against an OpenAPI schema, and routes them to JavaScript handler functions.

**Note**: This example uses **JavaScript ES2024** (not TypeScript) as specified in the Serverless Workflow DSL specification. Handlers use JSDoc comments for type documentation instead of TypeScript type annotations.

## Project Structure

```
javascript-openapi-listener/
├── Dockerfile                 # Container image definition
├── calculator-api.sw.yaml     # Serverless Workflow definition
├── openapi.spec.yaml          # OpenAPI 3.0 specification
├── deno.json                  # Deno configuration (includes axios dependency)
├── src/                       # JavaScript handler modules
│   ├── add.js                # Add operation handler
│   ├── multiply.js           # Multiply operation handler
│   └── get_pet.js            # Get pet handler (uses axios npm package)
└── README.md
```

## How It Works

### 1. HTTP Listeners

The workflow defines three HTTP listener tasks that bind to port `8080`:

```yaml
- handleAddRequests:
    listen:
      to:
        any:
          - with:
              source:
                uri: http://0.0.0.0:8080/api/v1/add
                schema:
                  format: openapi
                  resource:
                    endpoint: openapi.spec.yaml
                    name: AddRequest
        until: '${ false }'  # Listen forever
```

### 2. Schema Validation

Each request is validated against the OpenAPI schema defined in `openapi.spec.yaml`. The schema defines request/response types ensuring type safety at the API boundary.

### 3. Handler Execution

When a request arrives, the listener extracts the request data and passes it to the JavaScript handler:

```yaml
foreach:
  item: event
  do:
    - executeAdd:
        call: typescript  # Uses Deno runtime for ES2024 JavaScript
        with:
          module: src/add.js
          function: handler
          arguments:
            - ${ $event }
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
docker build -t calculator-js-api -f examples/javascript-openapi-listener/Dockerfile .
```

### Run the Container

```bash
docker run --rm -p 8080:8080 calculator-js-api
```

The HTTP server will start and listen on port `8080`.

## Testing the API

### Add Two Numbers

```bash
curl -X POST http://localhost:8080/api/v1/add \
  -H "Content-Type: application/json" \
  -d '{"a": 5, "b": 3}'
```

Expected response:
```json
{
  "result": 8
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

This endpoint demonstrates using third-party npm packages (`axios`) in JavaScript handlers:

```bash
curl http://localhost:8080/api/v1/pet/1
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
- **Third-party npm packages work** - Uses `axios` via Deno's npm: specifier
- Handlers can make external HTTP calls using popular libraries
- GET requests with path parameters work correctly
- OpenAPI schema validation works for all HTTP methods
- Dependencies are declared in `deno.json` and auto-installed by Deno

## Development

To work on the handlers locally:

1. Run the workflow directly (without Docker):
   ```bash
   jackdaw run calculator-api.sw.yaml --debug
   ```

2. Format JavaScript code:
   ```bash
   deno fmt src/
   ```

3. Lint JavaScript code:
   ```bash
   deno lint src/
   ```

## JavaScript ES2024 vs TypeScript

This example uses **JavaScript ES2024** as specified in the Serverless Workflow DSL specification, not TypeScript. Key differences:

- ✅ **No type annotations** - Uses JSDoc comments for documentation
- ✅ **No compilation step** - JavaScript runs directly
- ✅ **Spec compliant** - Follows DSL language version requirements
- ✅ **Built-in APIs** - Leverages ES2024 features like async/await, fetch, etc.

While Deno supports TypeScript, the spec requires JavaScript ES2024 for portability across runtimes.

## Production Considerations

For production deployments:

1. **Remove `--debug` flag** from the Dockerfile CMD
2. **Add health check endpoint** to the OpenAPI spec and workflow
3. **Configure HTTPS** with proper certificates
4. **Set resource limits** in your container orchestrator
5. **Add monitoring** and distributed tracing
6. **Implement rate limiting** for external API calls
7. **Add error handling** and retry logic for fetch operations
