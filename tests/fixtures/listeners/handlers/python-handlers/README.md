# Calculator Python Handlers

Strongly-typed Python handlers for the Calculator service, compatible with both gRPC and OpenAPI listeners.

## Project Structure

```
python-handlers/
├── pyproject.toml          # Project configuration (uv/pip)
├── calculator/             # Handler package
│   ├── __init__.py
│   ├── types.py           # Shared type definitions (TypedDict)
│   ├── add.py             # Add operation handler
│   └── multiply.py        # Multiply operation handler
└── README.md
```

## Type Safety

All handlers use Python `TypedDict` for strong typing:

- `AddRequest` / `AddResponse` - matching proto `AddRequest` / `AddResponse`
- `MultiplyRequest` / `MultiplyResponse` - matching proto `MultiplyRequest` / `MultiplyResponse`

## Installation

```bash
uv pip install -e .
```

## Usage

Handlers are discovered by the workflow engine based on the proto service definition:
- Service: `calculator.Calculator`
- Method: `Add` → handler in `calculator.add.handler`
- Method: `Multiply` → handler in `calculator.multiply.handler`

Each handler function signature:
```python
def handler(request: RequestType) -> ResponseType:
    ...
```
