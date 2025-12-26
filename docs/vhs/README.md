# VHS Animated GIF Files Needed

This directory contains VHS tape files for generating animated GIFs used in the README.

## Existing GIFs ✅
1. `hello-world.gif` - Basic hello world example
2. `hello-world-debug.gif` - Hello world with --debug flag
3. `hello-world-validate.gif` - Validating hello world workflow
4. `run-container.gif` - Running container with environment variables
5. `cache-debug.gif` - Caching demonstration

## Missing GIFs - Need VHS Tape Files 🎬

### High Priority (Core Features)

#### `run-python.tape`
```bash
jackdaw run examples/python/python-basics.sw.yaml
```
Shows Python script execution with factorial calculation and data processing.

#### `run-javascript.tape`
```bash
jackdaw run examples/javascript/javascript-basics.sw.yaml
```
Shows JavaScript execution with Fibonacci sequence calculation.

#### `persistence-demo.tape`
```bash
# First run - fails at step 3
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb --input '{"attempt": 1}'

# Second run - resumes and completes
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb --input '{"attempt": 2}'
```
Shows workflow persistence and resumption after failure.

#### `listener-openapi.tape`
```bash
# Start the listener
jackdaw run examples/python-openapi-listener/calculator-api.sw.yaml

# In another terminal, test it
curl -X POST http://localhost:8080/api/v1/add -H "Content-Type: application/json" -d '{"a": 5, "b": 3}'
curl -X POST http://localhost:8080/api/v1/multiply -H "Content-Type: application/json" -d '{"a": 4, "b": 7}'
curl http://localhost:8080/api/v1/pet/2
```
Shows HTTP OpenAPI listener handling requests.

#### `listener-grpc.tape`
```bash
# Start the listener
jackdaw run examples/python-grpc-listener/calculator-api.sw.yaml

# In another terminal, test it with grpcurl
grpcurl -plaintext -d '{"a": 5, "b": 3}' localhost:50051 calculator.Calculator/Add
grpcurl -plaintext -d '{"a": 4, "b": 7}' localhost:50051 calculator.Calculator/Multiply
```
Shows gRPC listener handling requests.

#### `executor-rest.tape`
```bash
jackdaw run examples/rest/rest-api.sw.yaml
```
Shows making REST API calls to external services (JSONPlaceholder).

### Medium Priority (Provider Variations)

These are variations showing different provider options. Lower priority since they demonstrate the same concepts with different backends.

#### `cache-provider-redb.tape`
```bash
jackdaw run examples/cache/cache.sw.yaml --cache-provider redb -i '{ "userData": "user-data-1"}'
# Run again with different input to show caching
jackdaw run examples/cache/cache.sw.yaml --cache-provider redb -i '{ "userData": "user-data-2"}'
```

#### `cache-provider-sqlite.tape`
```bash
jackdaw run examples/cache/cache.sw.yaml --cache-provider sqlite --sqlite-db-url=cache.sqlite -i '{ "userData": "user-data-1"}'
jackdaw run examples/cache/cache.sw.yaml --cache-provider sqlite --sqlite-db-url=cache.sqlite -i '{ "userData": "user-data-2"}'
```

#### `cache-provider-postgres.tape`
```bash
# Requires Postgres running
jackdaw run examples/cache/cache.sw.yaml --cache-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "userData": "user-data-1"}'
```

#### `persistence-provider-redb.tape`
```bash
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb -i '{ "attempt": 1 }'
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb -i '{ "attempt": 2 }'
```

#### `persistence-provider-sqlite.tape`
```bash
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider sqlite --sqlite-db-url=persistence.sqlite -i '{ "attempt": 1 }'
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider sqlite --sqlite-db-url=persistence.sqlite -i '{ "attempt": 2 }'
```

#### `persistence-provider-postgres.tape`
```bash
# Requires Postgres running
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "attempt": 1 }'
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "attempt": 2 }'
```

## VHS Tape File Template

Here's a template for creating VHS tape files:

```vhs
Output docs/vhs/example-name.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Catppuccin Mocha"

Type "# Example description"
Sleep 500ms
Enter

Type "jackdaw run examples/example.sw.yaml"
Sleep 500ms
Enter

Sleep 5s
```

## How to Generate GIFs

1. Install VHS: `brew install vhs` (or from https://github.com/charmbracelet/vhs)
2. Create a `.tape` file with the commands
3. Run: `vhs file.tape`
4. The GIF will be generated in the specified output location

## Notes

- All GIFs should show the command being run and the output
- Keep GIFs short (under 30 seconds if possible)
- Use consistent terminal theme and sizing
- Show both successful execution and relevant output
- For listener examples, show both starting the listener and making requests to it
