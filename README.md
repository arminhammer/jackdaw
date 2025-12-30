# jackdaw

`jackdaw` is a workflow execution engine for the [Serverless Workflow](https://serverlessworkflow.io/) specification. It supports durable execution of workflows through a persistence layer, as well as caching execution to prevent duplicate execution of expensive workflow tasks. `jackdaw` is written in Rust, and is designed for extensibility, performance, and ease of deployment into many execution contexts. 

`jackdaw` is distributed as a static binary, as well as a Docker container image. It is cross-compiled for Linux AMD64 & ARM64, as well as a MacOS Universal Binary. It does not have a server component that needs to be installed, and is a self-contained CLI tool. This should make it easy to run as a standalone workflow executor, as well as embedded as part of a larger program.

### Serverless Workflow DSL

Many modern software applications can be represented conceptually as a "workflow" (or [DAG](https://en.wikipedia.org/wiki/Directed_acyclic_graph)). Workflows are useful because they can help abstract away execution and integration details from the user's business logic. In addition, durable workflow execution engines have proven popular because they are able to handle the state management of long-running business processes. Unfortunately there are many workflow engines, and most have cumbersome server components that need to be deployed and maintained. Most have their own specific way of implementing workflows that make it hard to switch to other engines. Serverless Workflow is a fascinating DSL that attempts to provide a standard way of representing a workflow that is not tied to any particular engine implementation. 

Although `jackdaw` does not have a server component itself, it fully supports Serverless Workflow [Listeners](https://github.com/serverlessworkflow/specification/blob/main/dsl-reference.md#listen). This makes it possible to use `jackdaw` as a server that can trigger off of event types supported by Serverless Workflow, notably OpenAPI and gRPC specifications.

### Project Status

This project is committed to always being free and useful open source software under the standard Apache 2.0 license. There's no risk of vendor lock-in, because any workflow you run with `jackdaw` can be executed by any of the other Serverless Workflow runtimes!

A note on project stability: while the goal of this project is to support 100% of the Serverless Workflow specification, there are still gaps. The internals are unstable and subject to change as it is developed, but valid workflows should continue to run on every version.

If there are discrepancies between the Serverless Workflow spec and `jackdaw`, they should be considered bugs that will be resolved in favor of the spec. There may be features that are supported in `jackdaw` that are not supported in other engines, but they should not impact the portability of the workflows themselves.

## Getting started

Let's start off with the simplest Serverless Workflow we can imagine, a simple and classic Hello World:

```yaml
document:
  dsl: '1.0.2'
  name: hello-world
  namespace: examples
  version: '0.1.0'
output:
  as: .statement
do:
  - say:
      set:
        statement: Hello, World!
```

If we save this file as `hello-world.sw.yaml`, we can run it with the command

```
jackdaw run hello-world.sw.yaml
```

![Hello World](docs/vhs/hello-world.gif)

#### --debug flag

By default, `jackdaw` will hide verbose output. If you would like to see additional information to make it easier to troubleshoot what is happening, use the `--debug` flag:

```
jackdaw run hello-world.sw.yaml --debug
```

![Hello World](docs/vhs/hello-world-debug.gif)

## Installation

### Docker image

`jackdaw` is available as a container image from the releases page

### Download the binary

#### `jackdaw`

The `jackdaw` binary can be downloaded from the releases page. 

### From source

#### default version

The most straightforward way to install `jackdaw` is to clone the repository and run 

```bash
just build-static
```

This will compile the release binary.

## Usage

### `run`

#### container

`jackdaw` supports executing commands in containers. The default (and currently only) container runtime supported is Docker.

```yaml
document:
  dsl: '1.0.2'
  namespace: default
  name: test-env-vars
  version: '1.0.0'
do:
  - printEnv:
      run:
        container:
          image: alpine:latest
          command: sh -c "echo MY_VAR=$MY_VAR ANOTHER=$ANOTHER"
          environment:
            MY_VAR: "HelloWorld"
            ANOTHER: "TestValue"
```

```bash
jackdaw run examples/container/container-env-vars.sw.yaml
```
![Run Container](docs/vhs/run-container.gif)

Please make sure that a Docker socket at `/var/run/docker.sock` is available to `jackdaw` and a container runtime like Docker or Podman for this to work. This feature is implemented using the great [bollard](https://docs.rs/bollard/latest/bollard/) library.

#### Python

Python scripts are supported by `jackdaw`. The most straightforward way to use a python script is to embed the script directly in the workflow. See ['examples/python/python-basics.sw.yaml'](./examples/python/python-basics.sw.yaml).

```bash
jackdaw run examples/python/python-basics.sw.yaml
```

![Run Python](docs/vhs/run-python.gif)

Alternatively, you can also define a python module, with dependencies, and execute a function within that module. An example can be found in `examples/python-module`.

In order to execute a python script, the `python` binary must be available in the system PATH.

#### Javascript

Serverless Workflow support scripts written in Javascript ES2024. `jackdaw` supports javascript scripts by calling `node`, which must be present in the system PATH. This makes it possible to embed inlined javascript scripts into a workflow task:

```bash
jackdaw run examples/javascript/javascript-basics.sw.yaml
```

![Run JavaScript](docs/vhs/run-javascript.gif)

#### Nested workflows

Serverless Workflow can nest other workflows, making reuse very powerful. In the following example, Workflow A imports Workflow B, which in turn imports Workflow C.

Workflow C:
```yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: workflow-c
  version: '1.0.0'
do:
  - subtractTen:
      set:
        value: '${ .value - 10 }'
output:
  as: '${ . }'
```

Workflow B:
```yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: workflow-b
  version: '1.0.0'
do:
  - multiplyByTwo:
      set:
        value: '${ .value * 2 }'

  - callWorkflowC:
      run:
        workflow:
          namespace: examples
          name: workflow-c
          version: '1.0.0'
          input:
            value: '${ .value }'
output:
  as: '${ . }'
```

Workflow A:
```yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: workflow-a
  version: '1.0.0'
do:
  - addFive:
      set:
        value: '${ .value + 5 }'

  - callWorkflowB:
      run:
        workflow:
          namespace: examples
          name: workflow-b
          version: '1.0.0'
          input:
            value: '${ .value }'
output:
  as: '${ . }'
```

Execution:
```bash
jackdaw run examples/nested-workflows/workflow-a.yaml -i '{"value": 10}'
```

#### Tasks from a Catalog

Catalogs are collections of workflows, and act like reusable libraries. It is easy to define a new catalog and make it available for consumption within a workflow. `jackdaw` fully supports workflow catalogs.

To define a catalog:
```yaml
# examples/catalog/functions/add-numbers/1.0.0/function.yaml
document:
  dsl: '1.0.2'
  namespace: examples.functions
  name: add-numbers
  version: '1.0.0'
  description: 'Add two numbers together'
input:
  schema:
    type: object
    properties:
      a:
        type: number
      b:
        type: number
    required:
      - a
      - b
do:
  - add:
      set:
        result: '${ .a + .b }'
```

To consume a workflow from a catalog:
```yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: use-catalog
  version: '1.0.0'
use:
  catalogs:
    local:
      endpoint:
        uri: file://./examples/catalog/functions
do:
  - addNumbers:
      call: add-numbers:1.0.0
      with:
        a: 10
        b: 5
      output:
        as: '${ { addResult: .result } }'

  - multiplyNumbers:
      call: multiply-numbers:1.0.0
      with:
        a: '${ .addResult }'
        b: 3
```

```bash
jackdaw run examples/catalog/use-catalog.sw.yaml
```

#### Caching

Caching is a core feature of `jackdaw`. During execution, the input object of every task is hashed, and checked against the cache. If the same task was executed previously with the exact input object, then the cached output will be pulled from the cache and the task will not execute again. This can be quite useful when executing workflows with expensive tasks.

An example workflow that demonstrates the benefit of caching is the following `cache-demo` workflow. It has an expensive first task, which needs to be calculated but doesn't change based off of inputs. If it is not cached, then it has to be recalculated every single time. With caching enabled, it does not have to be recalculated, and the workflow moves to the other tasks quickly.

```yaml
document:
  dsl: '1.0.0'
  namespace: examples
  name: cache-demo
  version: '1.0.0'
  description: |
    Demonstrates caching behavior with an expensive computation.
    Step 1 performs an expensive calculation (simulated with sleep).
    Step 2 uses workflow input to process user data.
    Step 3 combines outputs from both steps.

    When run with different inputs:
    - First run: All steps execute
    - Second run with different input: Step 1 uses cached result, Steps 2 and 3 recalculate

do:
  # Step 1: Expensive computation that doesn't depend on input
  # This will be cached and reused across runs with different inputs
  - expensiveComputation:
      input:
        from: '{}'
      run:
        script:
          language: python
          code: |
            import time
            import hashlib

            time.sleep(5)

            result = {
                "computed_hash": hashlib.sha256(b"expensive-operation").hexdigest(),
                "dataset_size": 1000000,
                "processing_time": 5.0,
                "metadata": {
                    "algorithm": "sha256",
                    "iterations": 1000000
                }
            }

            print(result["computed_hash"])
      output:
        as: '${ { expensiveComputation: . } }'

  # Step 2: Process user input
  # This depends on workflow input, so it will recalculate when input changes
  - processUserInput:
      run: 
        script:
          language: python
          arguments:
            - ${ .userData }
            - ${ .expensiveComputation }
          code: |
            import sys
            import hashlib
            import json

            user_data = sys.argv[1]

            expensive_result = str(sys.argv[2])

            user_hash = hashlib.md5(user_data.encode()).hexdigest()

            result = {
                "user_hash": user_hash,
                "user_data_length": len(user_data),
                "processed": True,
                "expensive_computation": expensive_result
            }

            print(json.dumps(result))

  - combineResults:
      run:
        script:
          language: python
          arguments:
            - ${ .user_hash }
          code: |
            import sys
            import json
            user_hash = sys.argv[1]
            print(user_hash)
```

```bash
jackdaw run examples/cache/cache.sw.yaml -i '{ "userData": "user-data-1" }'
```

![Cache Debug](docs/vhs/cache-debug.gif)

#### Persistence

```yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: persistence-demo
  version: '1.0.0'
do:
  - step1:
      run:
        script:
          language: python
          code: |
            import json
            import time

            print("Step 1: Processing initial data...")
            time.sleep(1)

            result = {
                "step": 1,
                "status": "completed",
                "timestamp": time.time(),
                "data": "Important data from step 1"
            }

            print(json.dumps(result))
      output:
        as: '${ { step1: . } }'

  - step2:
      run:
        script:
          language: python
          arguments:
            - ${ .step1 }
          code: |
            import json
            import time
            import sys

            step1_data = json.loads(sys.argv[1])

            print("Step 2: Building on step 1 results...")
            time.sleep(1)

            result = {
                "step": 2,
                "status": "completed",
                "timestamp": time.time(),
                "data": "Processed data from step 2",
                "previous_data": step1_data["data"]
            }

            print(json.dumps(result))
      output:
        as: '${ { step2: . } }'

  - step3_mayFail:
      run:
        script:
          language: python
          arguments:
            - ${ .step2 }
            - ${ .attempt }
          code: |
            import json
            import time
            import sys

            step2_data = json.loads(sys.argv[1])
            attempt = int(sys.argv[2])

            print(f"Step 3: Attempting to complete workflow (attempt #{attempt})...")
            time.sleep(1)

            # Fail on first attempt to demonstrate persistence
            if attempt == 1:
                print("ERROR: Step 3 failed! This demonstrates workflow failure.")
                print("State has been persisted. Re-run with attempt=2 to resume.")
                sys.exit(1)

            # Succeed on second attempt
            result = {
                "step": 3,
                "status": "completed",
                "timestamp": time.time(),
                "data": "Final result after resuming from persisted state",
                "previous_data": step2_data["data"],
                "message": "Workflow completed successfully after resuming!"
            }

            print(json.dumps(result))
```

```bash
# First run (will fail at step 3, but persist state)
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb --input '{"attempt": 1}'

# Second run (will resume from persisted state and complete)
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb --input '{"attempt": 2}'
```

![Persistence Demo](docs/vhs/persistence-demo.gif)

#### Listeners

##### HTTP Listeners (OpenAPI)

```bash
# Python OpenAPI listener example
jackdaw run examples/python-openapi-listener/calculator-api.sw.yaml

# JavaScript OpenAPI listener example
jackdaw run examples/javascript-openapi-listener/calculator-api.sw.yaml
```

```bash
# Test the endpoints
curl -X POST http://localhost:8080/api/v1/add -H "Content-Type: application/json" -d '{"a": 5, "b": 3}'
curl -X POST http://localhost:8080/api/v1/multiply -H "Content-Type: application/json" -d '{"a": 4, "b": 7}'
```

<!-- ![OpenAPI Listener](docs/vhs/listener-openapi.gif) -->

##### gRPC Listeners

```bash
# Python gRPC listener example
jackdaw run examples/python-grpc-listener/calculator-api.sw.yaml

# JavaScript gRPC listener example
jackdaw run examples/javascript-grpc-listener/calculator-api.sw.yaml
```

<!-- ![gRPC Listener](docs/vhs/listener-grpc.gif) -->

### `validate`

```
jackdaw validate hello-world.sw.yaml
```

![Validate Command](docs/vhs/hello-world-validate.gif)

## Providers

### Cache Providers

#### in-memory

```bash
jackdaw run examples/cache/cache.sw.yaml --cache-provider memory -i '{ "userData": "user-data-1"}'
```

#### redb

```bash
jackdaw run examples/cache/cache.sw.yaml --cache-provider redb -i '{ "userData": "user-data-1"}'
```

#### sqlite

```bash
jackdaw run examples/cache/cache.sw.yaml --cache-provider sqlite --sqlite-db-url=cache.sqlite -i '{ "userData": "user-data-1"}'
```

#### postgres

```bash
jackdaw run examples/cache/cache.sw.yaml --cache-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "userData": "user-data-1"}'
```

### Persistence Providers

#### in-memory

```bash
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider memory -i '{ "attempt": 1 }'
```

#### redb

```bash
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb -i '{ "attempt": 1 }'
```

#### sqlite

```bash
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider sqlite --sqlite-db-url=persistence.sqlite -i '{ "attempt": 1 }'
```

#### postgres

```bash
jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "attempt": 1 }'
```

<!-- 
### Container Providers

#### Docker

### Executor Providers

#### OpenAPI

#### Python

#### Python External

#### Typescript

#### Typescript External -->

#### Rest

```yaml
document:
  dsl: '1.0.2'
  namespace: examples
  name: rest-api-calls
  version: '1.0.0'
do:
  - fetchUser:
      call: http
      with:
        method: get
        endpoint:
          uri: https://jsonplaceholder.typicode.com/users/1
      output:
        as: '${ { user: . } }'

  - fetchUserPosts:
      call: http
      with:
        method: get
        endpoint:
          uri: 'https://jsonplaceholder.typicode.com/posts?userId=${ .user.id }'
      output:
        as: '${ { posts: . } }'

  - summarizeData:
      run:
        script:
          language: python
          arguments:
            - ${ .user }
            - ${ .posts }
          code: |
            import sys
            import json

            user = json.loads(sys.argv[1])
            posts = json.loads(sys.argv[2])

            result = {
                "user_name": user["name"],
                "user_email": user["email"],
                "total_posts": len(posts),
                "post_titles": [post["title"] for post in posts]
            }

            print(json.dumps(result))
```

```bash
jackdaw run examples/rest/rest-api.sw.yaml
```

![REST API](docs/vhs/executor-rest.gif)

## Supported Serverless Features Matrix

## 1. Workflow Document Structure

### 1.1 Document Metadata

| Feature | Implementation |
|---------|----------------|
| `document.dsl` | ✅ Full |
| `document.namespace` | ✅ Full |
| `document.name` | ✅ Full |
| `document.version` | ✅ Full |
| `document.title` | ✅ Full |
| `document.summary` | ✅ Full |
| `document.tags` | ✅ Full |
| `document.metadata` | ✅ Full |

---

### 1.2 Top-Level Workflow Properties

| Feature | Implementation |
|---------|----------------|
| `input` | ✅ Full |
| `use` | ✅ Partial |
| `do` | ✅ Full |
| `timeout` | ✅ Full |
| `output` | ✅ Full |
| `schedule` | ❌ Not Implemented |

**Implementation Details:**

#### `use` Block Support:
- ✅ `use.functions` - Custom functions and workflows
- ✅ `use.catalogs` - External workflow catalogs
- ✅ `use.timeouts` - Reusable timeout policies
- ⚠️ `use.authentications` - Only basic auth supported
- ⚠️ `use.errors` - Error references not fully implemented
- ⚠️ `use.retries` - Retry policies recognized but limited testing
- ❌ `use.secrets` - No secret management system
- ❌ `use.extensions` - Not implemented

---

## 2. Task Types

### 2.1 Core Task Types Implementation

| Task Type | Implementation File | Notes |
|-----------|---------------------|-------|
| **call** | [tasks/call.rs](src/durableengine/tasks/call.rs) | HTTP, OpenAPI, Functions |
| **run** | [tasks/run.rs](src/durableengine/tasks/run.rs) | Container, Script, Shell, Workflow |
| **fork** | [tasks/fork.rs](src/durableengine/tasks/fork.rs) | Compete mode supported |
| **for** | [tasks/for_loop.rs](src/durableengine/tasks/for_loop.rs) | Item/index variables |
| **switch** | [tasks/switch.rs](src/durableengine/tasks/switch.rs) | Conditional branching |
| **try** | [tasks/try_catch.rs](src/durableengine/tasks/try_catch.rs) | Error filtering & catching |
| **emit** | [tasks/emit.rs](src/durableengine/tasks/emit.rs) | CloudEvents 1.0 |
| **raise** | [tasks/raise.rs](src/durableengine/tasks/raise.rs) | RFC 7807 errors |
| **wait** | [tasks/wait.rs](src/durableengine/tasks/wait.rs) | ISO 8601 durations |
| **set** | [tasks/mod.rs:217-255](src/durableengine/tasks/mod.rs) | Variable setting |
| **do** | [tasks/mod.rs:257-284](src/durableengine/tasks/mod.rs) | Sequential composition |
| **listen** | [tasks/mod.rs:286-332](src/durableengine/tasks/mod.rs) | Event consumption |

---

### 2.2 Task Base Properties

All task types inherit from `taskBase` with these common properties:

| Property | Implementation |
|----------|----------------|
| `if` | ✅ Full |
| `input` | ✅ Full |
| `output` | ✅ Full |
| `export` | ✅ Full |
| `timeout` | ✅ Full |
| `then` | ✅ Full |
| `metadata` | ✅ Full |

---

## 3. Call Task Sub-Types

### 3.1 Call Variants

| Call Type | Executor | Implementation |
|-----------|----------|----------------|
| **HTTP** | `RestExecutor` | ✅ Full |
| **OpenAPI** | `OpenApiExecutor` | ✅ Full |
| **gRPC** | - | ❌ Not Implemented |
| **AsyncAPI** | - | ❌ Not Implemented |
| **A2A** | - | ❌ Not Implemented |
| **MCP** | - | ❌ Not Implemented |
| **Function** | `catalog` lookup | ✅ Full |

---

### 3.2 HTTP Call Features

| Feature | Implementation |
|---------|----------------|
| HTTP Methods (GET/POST/PUT/DELETE) | ✅ Full |
| URI Templates | ✅ Full |
| Path Parameter Interpolation | ✅ Full |
| Headers | ✅ Full |
| Query Parameters | ✅ Full |
| Request Body | ✅ Full |
| Output Modes (content/response/raw) | ✅ Full |
| Redirect Handling | ✅ Full |
| Authentication | ⚠️ Basic Only |

**Output Modes:**
- `content` (default) - Response body only
- `response` - Full envelope with request metadata, headers, statusCode, content
- `raw` - Raw HTTP response

---

### 3.3 OpenAPI Call Features

| Feature | Implementation |
|---------|----------------|
| Document Loading (URI) | ✅ Full |
| Operation by operationId | ✅ Full |
| Parameter Mapping | ✅ Full |
| Output Modes | ✅ Full |
| Authentication | ❌ Not Implemented |
| Redirect Handling | ✅ Full |

**Supported OpenAPI Versions:**
- Swagger 2.0 ✅
- OpenAPI 3.x ✅

---

## 4. Run Task Execution Modes

### 4.1 Run Variants

| Run Mode | Implementation |
|----------|----------------|
| **Container** | ✅ Full |
| **Script** | ✅ Full |
| **Shell** | ✅ Full |
| **Workflow** | ✅ Full |

---

### 4.2 Container Execution

| Feature | Implementation |
|---------|----------------|
| Image Name | ✅ Full |
| Container Name | ✅ Full |
| Command Override | ✅ Full |
| Port Mappings | ✅ Full |
| Volume Mounts | ✅ Full |
| Environment Variables | ✅ Full |
| Stdin Input | ✅ Full |
| Arguments (argv) | ✅ Full |
| Lifetime/Cleanup Policy | ✅ Full |

**Cleanup Policies:**
- `always` - Remove after completion
- `never` - Keep running
- `eventually` - Remove after specified duration

---

### 4.3 Script Execution

| Feature | Implementation |
|---------|----------------|
| Language Selection | ✅ Full |
| Inline Code | ✅ Full |
| External Source (file://, http://, https://) | ✅ Full |
| Stdin Input | ✅ Full |
| Arguments (argv) | ✅ Full |
| Environment Variables | ✅ Full |

**Supported Languages:**
- **Python** - External executor via `PythonExecutor`
- **JavaScript** - External executor via `TypeScriptExecutor`

---

### 4.4 Shell Command Execution

| Feature | Implementation |
|---------|----------------|
| Command String | ✅ Full |
| Stdin Input | ✅ Full |
| Arguments (argv) | ✅ Full |
| Environment Variables | ✅ Full |

---

### 4.5 Run Task Common Features

| Feature | Implementation |
|---------|----------------|
| Await Process Completion | ✅ Full |
| Return Modes (stdout/stderr/code/all/none) | ✅ Full |
| Real-time Output Streaming | ✅ Full |
| Exit Code Validation | ✅ Full |

**Return Modes:**
- `stdout` (default) - Standard output only
- `stderr` - Standard error only
- `code` - Exit code only
- `all` - Combined { code, stdout, stderr }
- `none` - No output

---

## 5. Data Flow & Expressions

### 5.1 Expression Engine

| Feature | Implementation |
|---------|----------------|
| **JQ Expression Evaluation** | ✅ Full |
| **Null-Safe Field Access** | ✅ Full |
| **Null-Safe Array Operations** | ✅ Full |
| **Variable References ($var)** | ✅ Full |
| **String Interpolation** | ✅ Full |
| **Complex Expressions** | ✅ Full |

---

### 5.2 Input/Output Filtering

| Feature | Implementation |
|---------|----------------|
| **Workflow Input Schema** | ✅ Full |
| **Workflow Input Filtering** (`input.from`) | ✅ Full |
| **Task Input Schema** | ✅ Full |
| **Task Input Filtering** (`input.from`) | ✅ Full |
| **Task Output Schema** | ✅ Full |
| **Task Output Filtering** (`output.as`) | ✅ Full |
| **Workflow Output Schema** | ✅ Full |
| **Workflow Output Filtering** (`output.as`) | ✅ Full |

---

### 5.3 Export (Context Management)

| Feature | Implementation |
|---------|----------------|
| Export Schema | ✅ Full |
| Export Expression (`export.as`) | ✅ Full |
| Context Variable Storage | ✅ Full |

---

## 6. Flow Control

### 6.1 Flow Directives

| Directive | Implementation |
|-----------|----------------|
| **continue** | ✅ Full |
| **exit** | ✅ Full |
| **end** | ✅ Full |
| **Task Reference** (then: taskName) | ✅ Full |

---

### 6.2 Conditional Execution

| Feature | Implementation |
|---------|----------------|
| Task Condition (`if`) | ✅ Full |
| Switch Cases (`when`) | ✅ Full |
| Switch Default Case | ✅ Full |

---

## 7. Error Handling

### 7.1 Error Definition & Raising

| Feature | Implementation |
|---------|----------------|
| **Error Type (URI)** | ✅ Full |
| **Error Status** | ✅ Full |
| **Error Instance (JSON Pointer)** | ✅ Full |
| **Error Title** | ✅ Full |
| **Error Detail** | ✅ Full |
| **Error References** (`use.errors`) | ⚠️ Partial |

---

### 7.2 Error Catching & Recovery

| Feature | Implementation |
|---------|----------------|
| **Error Type Filtering** | ✅ Full |
| **Error Status Filtering** | ✅ Full |
| **Runtime Error Filtering** (`when`) | ✅ Full |
| **Error Variable Binding** (`as`) | ✅ Full |
| **Catch Handler Tasks** (`do`) | ✅ Full |
| **Retry Policies** | ⚠️ Partial |

---

### 7.3 Timeout Handling

| Feature | Implementation |
|---------|----------------|
| **Workflow Timeout** | ✅ Full |
| **Task Timeout** | ✅ Full |
| **Timeout Override** (Task > Workflow) | ✅ Full |
| **ISO 8601 Durations** | ✅ Full |
| **Inline Duration Objects** | ✅ Full |
| **Runtime Expression Durations** | ✅ Full |
| **Millisecond Precision** | ✅ Full |

---

## 8. Authentication & Security

### 8.1 Authentication Policies

| Auth Type | Implementation |
|-----------|----------------|
| **Basic Auth** | ✅ Full |
| **Bearer Auth** | ❌ Not Implemented |
| **Digest Auth** | ❌ Not Implemented |
| **OAuth2** | ❌ Not Implemented |
| **OIDC** | ❌ Not Implemented |

---

### 8.2 Secret Management

| Feature | Implementation |
|---------|----------------|
| **Secrets Declaration** (`use.secrets`) | ❌ Not Implemented |
| **Secret References** | ❌ Not Implemented |
| **Secret Vaulting** | ❌ Not Implemented |
| **Environment Variables** | ⚠️ Partial |

**Note:** Environment variables can be passed to containers/scripts, but no dedicated secret injection mechanism exists.

---

### 8.3 Authentication Integration

| Call Type | Auth Support | Status | Notes |
|-----------|-------------|--------|-------|
| HTTP/REST | Basic Auth | ✅ Implemented | Via `endpoint.authentication.basic` |
| OpenAPI | - | ❌ None | Security schemes ignored |
| gRPC | - | ❌ None | Not implemented |
| AsyncAPI | - | ❌ None | Not implemented |

---

## 9. Listeners & Event Consumption

### 9.1 Listen Task

| Feature | Implementation |
|---------|----------------|
| **Event Consumption Strategies** | ✅ Full |
| **Event Filters** | ✅ Full |
| **Read Modes** (data/envelope/raw) | ✅ Full |
| **Foreach Iterator** | ✅ Full |
| **Until Condition** | ✅ Full |

**Read Modes:**
- `data` - Extract CloudEvent data field only
- `envelope` - Full CloudEvent structure (default)
- `raw` - Raw HTTP body

---

### 9.2 Event Consumption Strategies

| Strategy | Implementation |
|----------|----------------|
| **One** - Single event | ✅ Full |
| **All** - All specified events | ✅ Full |
| **Any** - Any of specified events | ✅ Full |

---

### 9.3 Listener Implementations

| Listener Type | Implementation |
|---------------|----------------|
| **HTTP/OpenAPI** | ✅ Full |
| **gRPC** | ✅ Full |

---

### 9.4 Event Emission (Emit Task)

| Feature | Implementation |
|---------|----------------|
| **CloudEvents 1.0 Format** | ✅ Full |
| **Event Properties** (id, source, type, etc.) | ✅ Full |
| **Auto ID Generation** | ✅ Full |
| **Timestamp Generation** | ✅ Full |
| **Expression Evaluation** | ✅ Full |

---

## 10. Advanced Features

### 10.1 Nested Workflows

| Feature | Implementation |
|---------|----------------|
| **Workflow References** (namespace/name/version) | ✅ Full |
| **Input Passing** | ✅ Full |
| **Latest Version Resolution** | ✅ Full |

**Workflow Events:**
- ✅ WorkflowStarted
- ✅ WorkflowCompleted
- ✅ WorkflowFailed
- ✅ WorkflowCancelled
- ✅ WorkflowSuspended
- ✅ WorkflowResumed

**Task Events:**
- ✅ TaskCreated
- ✅ TaskStarted
- ✅ TaskCompleted
- ✅ TaskRetried
- ✅ TaskFaulted
- ✅ TaskCancelled
- ✅ TaskSuspended
- ✅ TaskResumed

---

## Feature Roadmap

- Full compliance with the Serverless Workflow specification
- A2A support
- AsyncAPI support
- MCP support
- Authentication and Secrets integrations
- AWS Lambda integration
- Kubernetes integration
- Workflow execution visualization with D2 and graphviz
- Import & Export from other workflow specifications, like Argo Workflows and Kubeflow Pipelines
- Native `jackdaw` bindings for [sdk-typescript](https://github.com/serverlessworkflow/sdk-typescript)
- Native `jackdaw` bindings for [sdk-python](https://github.com/serverlessworkflow/sdk-python)
- Complete OpenTelemetry-compatible instrumentation and metrics