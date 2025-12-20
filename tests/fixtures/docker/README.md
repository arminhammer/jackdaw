# Docker Integration Test Fixtures

This directory contains workflow fixtures specifically designed for Docker integration testing.

## Test Categories

### 1. Basic Workflows
- **valid-minimal.sw.yaml** - Minimal valid workflow for testing validation
- **invalid.sw.yaml** - Invalid workflow to test validation failures
- **simple-echo.sw.yaml** - Simple echo workflow for basic execution testing

### 2. Script Execution Tests
- **python-script.sw.yaml** - Tests Python code execution in Docker
- **typescript-script.sw.yaml** - Tests TypeScript code execution in Docker
- **javascript-script.sw.yaml** - Tests JavaScript code execution in Docker

### 3. Listener Tests (gRPC and OpenAPI)
- **grpc-python-calculator.sw.yaml** - gRPC listener with Python handler
- **grpc-typescript-calculator.sw.yaml** - gRPC listener with TypeScript handler
- **openapi-python-calculator.sw.yaml** - OpenAPI/HTTP listener with Python handler
- **openapi-typescript-calculator.sw.yaml** - OpenAPI/HTTP listener with TypeScript handler

## Running the Tests

### Run all Docker integration tests (validation only):
```bash
just test-docker
```

This runs the fast validation tests that verify workflows can be parsed and validated in Docker.

### Build Docker image and run validation tests:
```bash
just test-docker-full
```

### Run end-to-end listener tests with actual client connections:
```bash
# Run all ignored tests (requires grpcurl and curl)
cargo test --test docker_integration_tests -- --ignored

# Run specific listener test
cargo test --test docker_integration_tests test_docker_run_grpc_python_listener_with_client -- --ignored
cargo test --test docker_integration_tests test_docker_run_openapi_python_listener_with_client -- --ignored
```

**Prerequisites for end-to-end tests:**
- `curl` (usually pre-installed)
- `grpcurl` (install with `brew install grpcurl` or `apt-get install grpcurl`)

### Run specific validation tests:
```bash
cargo test --test docker_integration_tests test_docker_validate_python_script
```

## Test Structure

All tests in `tests/docker_integration_tests.rs` follow this pattern:

### 1. **Validation Tests** (Always run)
- Test that workflows validate correctly in Docker
- Fast and require no external dependencies
- Run automatically with `just test-docker`

### 2. **Execution Tests** (Marked `#[ignore]`)

#### Script Execution Tests:
- `test_docker_run_python_script` - Runs Python code in Docker
- `test_docker_run_typescript_script` - Runs TypeScript code in Docker
- Require database persistence

#### End-to-End Listener Tests:
- **`test_docker_run_grpc_python_listener_with_client`** - Starts gRPC listener in Docker, makes actual gRPC calls
- **`test_docker_run_openapi_python_listener_with_client`** - Starts HTTP listener in Docker, makes actual HTTP calls
- **`test_docker_run_grpc_typescript_listener_with_client`** - gRPC with TypeScript handler
- **`test_docker_run_openapi_typescript_listener_with_client`** - HTTP with TypeScript handler

These tests:
1. Start jackdaw in Docker with listener endpoints exposed
2. Wait for listeners to start (5 seconds)
3. Make actual client connections (via `grpcurl` or `curl`)
4. Verify responses
5. Clean up containers

#### Nested Workflow Tests:
- Test workflows that call other workflows
- Require file system access for workflow resolution

## Adding New Tests

To add a new Docker integration test:

1. Create a workflow fixture in this directory
2. Add a test function in `tests/docker_integration_tests.rs`
3. Use the existing patterns for consistency
4. Mark execution tests with `#[ignore]` if they require runtime services

## Dependencies

The listener workflows reference:
- `tests/fixtures/listeners/specs/calculator.proto` - gRPC proto definition
- `tests/fixtures/listeners/specs/calculator.yaml` - OpenAPI specification

These files must exist for validation to succeed.