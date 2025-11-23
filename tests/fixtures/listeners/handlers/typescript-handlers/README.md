# Calculator TypeScript Handlers

Strongly-typed TypeScript handlers for the Calculator service, compatible with both gRPC and OpenAPI listeners.

## Project Structure

```
typescript-handlers/
├── deno.json              # Deno configuration
├── src/
│   ├── types.ts          # Shared type definitions (interfaces)
│   ├── add.ts            # Add operation handler
│   └── multiply.ts       # Multiply operation handler
└── README.md
```

## Type Safety

All handlers use TypeScript interfaces for strong typing:

- `AddRequest` / `AddResponse` - matching proto `AddRequest` / `AddResponse`
- `MultiplyRequest` / `MultiplyResponse` - matching proto `MultiplyRequest` / `MultiplyResponse`

## Requirements

- Deno runtime

## Type Checking

```bash
deno check src/*.ts
```

## Linting

```bash
deno lint
```

## Formatting

```bash
deno fmt
```

## Usage

Handlers are discovered by the workflow engine based on the proto service definition:
- Service: `calculator.Calculator`
- Method: `Add` → handler in `src/add.ts` exported as `handler`
- Method: `Multiply` → handler in `src/multiply.ts` exported as `handler`

Each handler function signature:
```typescript
export function handler(request: RequestType): ResponseType {
  ...
}
```

## Integration

The Deno runtime loads these modules and calls the exported `handler` function.
Type safety is enforced at both compile-time (TypeScript) and runtime (Deno).
