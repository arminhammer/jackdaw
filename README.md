# jackdaw

`jackdaw` is a workflow execution engine for the [Serverless Workflow](https://serverlessworkflow.io/) specification. It supports durable execution of workflows through a persistence layer, as well as caching execution to prevent duplicate execution of expensive workflow tasks. `jackdaw` is written in Rust, and is designed for extensibiliy, performance, and ease of deploymeny into many execution contexts. 

`jackdaw` is distributed as a static binary, as well as a Docker container image. It is cross-compiled for Linux AMD64 & ARM64, as well as a MacOS Universal Binary. It does not have a server component that needs to be installed, and is a self-contained CLI tool. This should make it easy to run as a standalone workflow executor, as well as embedded as part of a larger program.

### Serverless Workflow DSL

Many modern software applications can be represented conceptually as a "workflow" (or [DAG](https://en.wikipedia.org/wiki/Directed_acyclic_graph)). Workflows are useful because they can help abstract away execution and integration details from the user's business logic. In addition, durable worklow execution engines have proven popular because they are able to handle the state management of long-running business processes. Unfortunately there are many workflow engines, and most have cumbersome server components that need to be deployed and maintained. Most have their own specific way of implementing workflows that make it hard to switch to other engines. Serverless Workflow is a fascinating DSL that attempts to provide a standard way of representing a workflow that is not tied to any particular engine implementation. 

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

![OpenAPI Listener](docs/vhs/listener-openapi.gif)

##### gRPC Listeners

```bash
# Python gRPC listener example
jackdaw run examples/python-grpc-listener/calculator-api.sw.yaml

# JavaScript gRPC listener example
jackdaw run examples/javascript-grpc-listener/calculator-api.sw.yaml
```

![gRPC Listener](docs/vhs/listener-grpc.gif)

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

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| `document.dsl` | Required | ✅ Full | **Complete** | ✅ CTK + Validation |
| `document.namespace` | Required | ✅ Full | **Complete** | ✅ CTK + Validation |
| `document.name` | Required | ✅ Full | **Complete** | ✅ CTK + Validation |
| `document.version` | Required | ✅ Full | **Complete** | ✅ CTK + Validation |
| `document.title` | Optional | ✅ Full | **Complete** | ✅ Validation |
| `document.summary` | Optional | ✅ Full | **Complete** | ✅ Validation |
| `document.tags` | Optional | ✅ Full | **Complete** | ✅ Validation |
| `document.metadata` | Optional | ✅ Full | **Complete** | ✅ Validation |

---

### 1.2 Top-Level Workflow Properties

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| `input` | Schema + Filtering | ✅ Full | **Complete** | ✅ CTK Data Flow |
| `use` | Reusable Components | ✅ Partial | **Beta** | ✅ Functions, ⚠️ Limited Auth |
| `do` | Task List | ✅ Full | **Complete** | ✅ CTK All Features |
| `timeout` | Duration | ✅ Full | **Complete** | ✅ 11 Integration Tests |
| `output` | Schema + Filtering | ✅ Full | **Complete** | ✅ CTK Data Flow |
| `schedule` | Trigger Config | ❌ Not Implemented | **Not Started** | ❌ None |

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

| Task Type | Spec Section | Implementation File | Maturity | CTK Tests | Unit Tests | Notes |
|-----------|-------------|---------------------|----------|-----------|------------|-------|
| **call** | §223-595 | [tasks/call.rs](src/durableengine/tasks/call.rs) | ✅ **Full** | 5 scenarios | - | HTTP, OpenAPI, Functions |
| **run** | §752-933 | [tasks/run.rs](src/durableengine/tasks/run.rs) | ✅ **Full** | - | 7+ tests | Container, Script, Shell, Workflow |
| **fork** | §596-619 | [tasks/fork.rs](src/durableengine/tasks/fork.rs) | ✅ **Full** | 1 scenario | - | Compete mode supported |
| **for** | §660-696 | [tasks/for_loop.rs](src/durableengine/tasks/for_loop.rs) | ✅ **Full** | 1 scenario | - | Item/index variables |
| **switch** | §951-985 | [tasks/switch.rs](src/durableengine/tasks/switch.rs) | ✅ **Full** | 3 scenarios | 1 test | Conditional branching |
| **try** | §986-1034 | [tasks/try_catch.rs](src/durableengine/tasks/try_catch.rs) | ✅ **Full** | 2 scenarios | - | Error filtering & catching |
| **emit** | §633-659 | [tasks/emit.rs](src/durableengine/tasks/emit.rs) | ✅ **Full** | 1 scenario | - | CloudEvents 1.0 |
| **raise** | §728-751 | [tasks/raise.rs](src/durableengine/tasks/raise.rs) | ✅ **Full** | 1 scenario | - | RFC 7807 errors |
| **wait** | §1035-1047 | [tasks/wait.rs](src/durableengine/tasks/wait.rs) | ✅ **Full** | - | 7 tests | ISO 8601 durations |
| **set** | §934-950 | [tasks/mod.rs:217-255](src/durableengine/tasks/mod.rs) | ✅ **Full** | 1 scenario | - | Variable setting |
| **do** | §620-632 | [tasks/mod.rs:257-284](src/durableengine/tasks/mod.rs) | ✅ **Full** | 1 scenario | - | Sequential composition |
| **listen** | §697-727 | [tasks/mod.rs:286-332](src/durableengine/tasks/mod.rs) | ✅ **Full** | - | 4 tests | Event consumption |

---

### 2.2 Task Base Properties

All task types inherit from `taskBase` with these common properties:

| Property | Spec Ref | Implementation | Maturity | Test Coverage |
|----------|----------|----------------|----------|---------------|
| `if` | §172-175 | ✅ Full | **Complete** | ✅ Implicit in CTK |
| `input` | §176-179 | ✅ Full | **Complete** | ✅ CTK Data Flow |
| `output` | §180-183 | ✅ Full | **Complete** | ✅ CTK Data Flow |
| `export` | §184-187 | ✅ Full | **Complete** | ✅ Integration Tests |
| `timeout` | §188-196 | ✅ Full | **Complete** | ✅ 11 Timeout Tests |
| `then` | §197-200 | ✅ Full | **Complete** | ✅ CTK Flow |
| `metadata` | §201-205 | ✅ Full | **Complete** | ✅ Validation |

---

## 3. Call Task Sub-Types

### 3.1 Call Variants

| Call Type | Spec Section | Executor | Implementation | Maturity | Test Coverage |
|-----------|-------------|----------|----------------|----------|---------------|
| **HTTP** | §338-392 | `RestExecutor` | ✅ Full | **Complete** | ✅ 3 CTK scenarios + 3 redirect tests |
| **OpenAPI** | §393-436 | `OpenApiExecutor` | ✅ Full | **Complete** | ✅ 2 CTK scenarios |
| **gRPC** | §282-337 | - | ❌ Not Implemented | **Not Started** | ❌ None |
| **AsyncAPI** | §227-281 | - | ❌ Not Implemented | **Not Started** | ❌ None |
| **A2A** | §437-475 | - | ❌ Not Implemented | **Not Started** | ❌ None |
| **MCP** | §476-577 | - | ❌ Not Implemented | **Not Started** | ❌ None |
| **Function** | §578-595 | `catalog` lookup | ✅ Full | **Complete** | ✅ Implicit in examples |

---

### 3.2 HTTP Call Features

| Feature | Spec Ref | Implementation | Maturity | Test Coverage |
|---------|----------|----------------|----------|---------------|
| HTTP Methods (GET/POST/PUT/DELETE) | §354 | ✅ Full | **Complete** | ✅ CTK |
| URI Templates | §359 | ✅ Full | **Complete** | ✅ CTK |
| Path Parameter Interpolation | § | ✅ Full | **Complete** | ✅ CTK |
| Headers | §362-369 | ✅ Full | **Complete** | ✅ CTK |
| Query Parameters | §373-381 | ✅ Full | **Complete** | ✅ CTK |
| Request Body | §370-372 | ✅ Full | **Complete** | ✅ CTK |
| Output Modes (content/response/raw) | §382-386 | ✅ Full | **Complete** | ✅ CTK |
| Redirect Handling | §387-390 | ✅ Full | **Complete** | ✅ 3 Integration Tests |
| Authentication | §361 | ⚠️ Basic Only | **Beta** | ✅ 1 CTK scenario |

**Output Modes:**
- `content` (default) - Response body only
- `response` - Full envelope with request metadata, headers, statusCode, content
- `raw` - Raw HTTP response

---

### 3.3 OpenAPI Call Features

| Feature | Spec Ref | Implementation | Maturity | Test Coverage |
|---------|----------|----------------|----------|---------------|
| Document Loading (URI) | §409-412 | ✅ Full | **Complete** | ✅ CTK |
| Operation by operationId | §413-416 | ✅ Full | **Complete** | ✅ CTK |
| Parameter Mapping | §417-421 | ✅ Full | **Complete** | ✅ CTK |
| Output Modes | §426-430 | ✅ Full | **Complete** | ✅ CTK |
| Authentication | §422-425 | ❌ Not Implemented | **Not Started** | ❌ None |
| Redirect Handling | §431-434 | ✅ Full | **Complete** | ✅ Inherited from HTTP |

**Supported OpenAPI Versions:**
- Swagger 2.0 ✅
- OpenAPI 3.x ✅

---

## 4. Run Task Execution Modes

### 4.1 Run Variants

| Run Mode | Spec Section | Implementation | Maturity | Test Coverage |
|----------|-------------|----------------|----------|---------------|
| **Container** | §779-826 | ✅ Full | **Complete** | ✅ Integration Tests |
| **Script** | §828-873 | ✅ Full | **Complete** | ✅ CTK Examples |
| **Shell** | §875-903 | ✅ Full | **Complete** | ✅ Integration Tests |
| **Workflow** | §905-933 | ✅ Full | **Complete** | ✅ Nested Workflow Tests |

---

### 4.2 Container Execution

| Feature | Spec Ref | Implementation | Maturity | Test Coverage |
|---------|----------|----------------|----------|---------------|
| Image Name | §788-791 | ✅ Full | **Complete** | ✅ Tests |
| Container Name | §792-795 | ✅ Full | **Complete** | ✅ Tests |
| Command Override | §796-799 | ✅ Full | **Complete** | ✅ Tests |
| Port Mappings | §800-803 | ✅ Full | **Complete** | ✅ Tests |
| Volume Mounts | §804-807 | ✅ Full | **Complete** | ✅ Tests |
| Environment Variables | §808-811 | ✅ Full | **Complete** | ✅ 1 Integration Test |
| Stdin Input | §812-815 | ✅ Full | **Complete** | ✅ CTK Example |
| Arguments (argv) | §816-821 | ✅ Full | **Complete** | ✅ CTK Example |
| Lifetime/Cleanup Policy | §822-825 | ✅ Full | **Complete** | ✅ Tests |

**Cleanup Policies:**
- `always` - Remove after completion
- `never` - Keep running
- `eventually` - Remove after specified duration

---

### 4.3 Script Execution

| Feature | Spec Ref | Implementation | Maturity | Test Coverage |
|---------|----------|----------------|----------|---------------|
| Language Selection | §838-840 | ✅ Full | **Complete** | ✅ Examples |
| Inline Code | §858-863 | ✅ Full | **Complete** | ✅ Examples |
| External Source (file://, http://, https://) | §865-871 | ✅ Full | **Complete** | ✅ Examples |
| Stdin Input | §841-844 | ✅ Full | **Complete** | ✅ Examples |
| Arguments (argv) | §845-850 | ✅ Full | **Complete** | ✅ Examples |
| Environment Variables | §851-855 | ✅ Full | **Complete** | ✅ Examples |

**Supported Languages:**
- **Python** - External executor via `PythonExecutor`
- **JavaScript** - External executor via `TypeScriptExecutor`

---

### 4.4 Shell Command Execution

| Feature | Spec Ref | Implementation | Maturity | Test Coverage |
|---------|----------|----------------|----------|---------------|
| Command String | §884-887 | ✅ Full | **Complete** | ✅ Tests |
| Stdin Input | §888-891 | ✅ Full | **Complete** | ✅ Tests |
| Arguments (argv) | §892-897 | ✅ Full | **Complete** | ✅ Tests |
| Environment Variables | §898-902 | ✅ Full | **Complete** | ✅ Tests |

---

### 4.5 Run Task Common Features

| Feature | Spec Ref | Implementation | Maturity | Test Coverage |
|---------|----------|----------------|----------|---------------|
| Await Process Completion | §768-771 | ✅ Full | **Complete** | ✅ Tests |
| Return Modes (stdout/stderr/code/all/none) | §772-777 | ✅ Full | **Complete** | ✅ Tests |
| Real-time Output Streaming | - | ✅ Full | **Complete** | ✅ Tests |
| Exit Code Validation | - | ✅ Full | **Complete** | ✅ Tests |

**Return Modes:**
- `stdout` (default) - Standard output only
- `stderr` - Standard error only
- `code` - Exit code only
- `all` - Combined { code, stdout, stderr }
- `none` - No output

---

## 5. Data Flow & Expressions

### 5.1 Expression Engine

| Feature | Implementation | Maturity | Test Coverage |
|---------|----------------|----------|---------------|
| **JQ Expression Evaluation** | ✅ Full | **Complete** | ✅ CTK Data Flow |
| **Null-Safe Field Access** | ✅ Full | **Complete** | ✅ Unit Tests |
| **Null-Safe Array Operations** | ✅ Full | **Complete** | ✅ Unit Tests |
| **Variable References ($var)** | ✅ Full | **Complete** | ✅ CTK |
| **String Interpolation** | ✅ Full | **Complete** | ✅ CTK |
| **Complex Expressions** | ✅ Full | **Complete** | ✅ CTK |

---

### 5.2 Input/Output Filtering

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Workflow Input Schema** | §1613-1629 | ✅ Full | **Complete** | ✅ Validation |
| **Workflow Input Filtering** (`input.from`) | §1623-1628 | ✅ Full | **Complete** | ✅ CTK Data Flow |
| **Task Input Schema** | §176-179 | ✅ Full | **Complete** | ✅ Tests |
| **Task Input Filtering** (`input.from`) | §176-179 | ✅ Full | **Complete** | ✅ 1 CTK Scenario |
| **Task Output Schema** | §180-183 | ✅ Full | **Complete** | ✅ Tests |
| **Task Output Filtering** (`output.as`) | §180-183 | ✅ Full | **Complete** | ✅ 2 CTK Scenarios |
| **Workflow Output Schema** | §1630-1645 | ✅ Full | **Complete** | ✅ Validation |
| **Workflow Output Filtering** (`output.as`) | §1639-1644 | ✅ Full | **Complete** | ✅ CTK |

---

### 5.3 Export (Context Management)

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| Export Schema | §1646-1661 | ✅ Full | **Complete** | ✅ Tests |
| Export Expression (`export.as`) | §1655-1660 | ✅ Full | **Complete** | ✅ Tests |
| Context Variable Storage | - | ✅ Full | **Complete** | ✅ Tests |

---

## 6. Flow Control

### 6.1 Flow Directives

| Directive | Spec Section | Implementation | Maturity | Test Coverage |
|-----------|-------------|----------------|----------|---------------|
| **continue** | §1048-1055 | ✅ Full | **Complete** | ✅ CTK Flow |
| **exit** | §1048-1055 | ✅ Full | **Complete** | ✅ 1 Integration Test |
| **end** | §1048-1055 | ✅ Full | **Complete** | ✅ 1 Integration Test |
| **Task Reference** (then: taskName) | §197-200 | ✅ Full | **Complete** | ✅ CTK Flow |

---

### 6.2 Conditional Execution

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| Task Condition (`if`) | §172-175 | ✅ Full | **Complete** | ✅ Implicit |
| Switch Cases (`when`) | §976-980 | ✅ Full | **Complete** | ✅ 3 CTK Scenarios |
| Switch Default Case | §968-973 | ✅ Full | **Complete** | ✅ 2 CTK Scenarios |

---

## 7. Error Handling

### 7.1 Error Definition & Raising

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Error Type (URI)** | §1352-1362 | ✅ Full | **Complete** | ✅ CTK |
| **Error Status** | §1363-1366 | ✅ Full | **Complete** | ✅ CTK |
| **Error Instance (JSON Pointer)** | §1367-1377 | ✅ Full | **Complete** | ✅ CTK |
| **Error Title** | §1378-1385 | ✅ Full | **Complete** | ✅ CTK |
| **Error Detail** | §1386-1393 | ✅ Full | **Complete** | ✅ CTK |
| **Error References** (`use.errors`) | §68-73 | ⚠️ Partial | **Beta** | ❌ None |

---

### 7.2 Error Catching & Recovery

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Error Type Filtering** | §1004-1010 | ✅ Full | **Complete** | ✅ 2 CTK Scenarios |
| **Error Status Filtering** | §1004-1010 | ✅ Full | **Complete** | ✅ CTK |
| **Runtime Error Filtering** (`when`) | §1015-1018 | ✅ Full | **Complete** | ✅ Tests |
| **Error Variable Binding** (`as`) | §1011-1014 | ✅ Full | **Complete** | ✅ CTK |
| **Catch Handler Tasks** (`do`) | §1031-1034 | ✅ Full | **Complete** | ✅ CTK |
| **Retry Policies** | §1023-1030 | ⚠️ Partial | **Beta** | ⚠️ Limited |

---

### 7.3 Timeout Handling

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Workflow Timeout** | §120-128 | ✅ Full | **Complete** | ✅ 2 Tests |
| **Task Timeout** | §188-196 | ✅ Full | **Complete** | ✅ 2 Tests |
| **Timeout Override** (Task > Workflow) | - | ✅ Full | **Complete** | ✅ 1 Test |
| **ISO 8601 Durations** | §1312-1346 | ✅ Full | **Complete** | ✅ Tests |
| **Inline Duration Objects** | §1314-1338 | ✅ Full | **Complete** | ✅ Tests |
| **Runtime Expression Durations** | §1340-1342 | ✅ Full | **Complete** | ✅ Tests |
| **Millisecond Precision** | - | ✅ Full | **Complete** | ✅ 1 Test |

---

## 8. Authentication & Security

### 8.1 Authentication Policies

| Auth Type | Spec Section | Implementation | Maturity | Test Coverage |
|-----------|-------------|----------------|----------|---------------|
| **Basic Auth** | §1090-1112 | ✅ Full | **Complete** | ✅ 1 CTK Scenario |
| **Bearer Auth** | §1113-1132 | ❌ Not Implemented | **Not Started** | ❌ None |
| **Digest Auth** | §1133-1155 | ❌ Not Implemented | **Not Started** | ❌ None |
| **OAuth2** | §1156-1197 | ❌ Not Implemented | **Not Started** | ❌ None |
| **OIDC** | §1198-1214 | ❌ Not Implemented | **Not Started** | ❌ None |

---

### 8.2 Secret Management

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Secrets Declaration** (`use.secrets`) | §98-103 | ❌ Not Implemented | **Not Started** | ❌ None |
| **Secret References** | - | ❌ Not Implemented | **Not Started** | ❌ None |
| **Secret Vaulting** | - | ❌ Not Implemented | **Not Started** | ❌ None |
| **Environment Variables** | - | ⚠️ Partial | **Beta** | ✅ Container Tests |

**Note:** Environment variables can be passed to containers/scripts but no dedicated secret injection mechanism exists.

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

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Event Consumption Strategies** | §1504-1545 | ✅ Full | **Complete** | ✅ Tests |
| **Event Filters** | §1546-1573 | ✅ Full | **Complete** | ✅ Tests |
| **Read Modes** (data/envelope/raw) | §716-721 | ✅ Full | **Complete** | ✅ 4 Integration Tests |
| **Foreach Iterator** | §723-726 | ✅ Full | **Complete** | ✅ Tests |
| **Until Condition** | - | ✅ Full | **Complete** | ✅ Tests |

**Read Modes:**
- `data` - Extract CloudEvent data field only
- `envelope` - Full CloudEvent structure (default)
- `raw` - Raw HTTP body

---

### 9.2 Event Consumption Strategies

| Strategy | Spec Section | Implementation | Maturity | Test Coverage |
|----------|-------------|----------------|----------|---------------|
| **One** - Single event | §1539-1545 | ✅ Full | **Complete** | ✅ Tests |
| **All** - All specified events | §1510-1518 | ✅ Full | **Complete** | ✅ Tests |
| **Any** - Any of specified events | §1519-1538 | ✅ Full | **Complete** | ✅ Tests |

---

### 9.3 Listener Implementations

| Listener Type | Spec Section | Implementation | Maturity | Test Coverage |
|---------------|-------------|----------------|----------|---------------|
| **HTTP/OpenAPI** | - | ✅ Full | **Complete** | ✅ 2 Feature Tests |
| **gRPC** | - | ✅ Full | **Complete** | ✅ 2 Feature Tests |

---

### 9.4 Event Emission (Emit Task)

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **CloudEvents 1.0 Format** | §648-658 | ✅ Full | **Complete** | ✅ 1 CTK Scenario |
| **Event Properties** (id, source, type, etc.) | §1452-1503 | ✅ Full | **Complete** | ✅ CTK |
| **Auto ID Generation** | - | ✅ Full | **Complete** | ✅ CTK |
| **Timestamp Generation** | - | ✅ Full | **Complete** | ✅ CTK |
| **Expression Evaluation** | - | ✅ Full | **Complete** | ✅ CTK |

---

## 10. Advanced Features

### 10.1 Nested Workflows

| Feature | Spec Section | Implementation | Maturity | Test Coverage |
|---------|-------------|----------------|----------|---------------|
| **Workflow References** (namespace/name/version) | §905-933 | ✅ Full | **Complete** | ✅ Feature Test |
| **Input Passing** | §927-931 | ✅ Full | **Complete** | ✅ Tests |
| **Latest Version Resolution** | §922-926 | ✅ Full | **Complete** | ✅ Tests |

---

### 10.2 Caching

| Feature | Implementation | Maturity | Test Coverage |
|---------|----------------|----------|---------------|
| **Hash-Based Caching** | ✅ Full | **Complete** | ✅ Implicit |
| **Cache Key Computation** | ✅ Full | **Complete** | ✅ Implicit |
| **Multiple Backends** | ✅ Full | **Complete** | ✅ Implicit |

---

### 10.3 Persistence & Durability

| Feature | Implementation | Maturity | Test Coverage |
|---------|----------------|----------|---------------|
| **Event Sourcing** | ✅ Full | **Complete** | ✅ 6 Event Tests |
| **State Snapshots** | ✅ Full | **Complete** | ✅ Implicit |
| **Recovery** | ✅ Full | **Complete** | ✅ Implicit |
| **Multiple Backends** | ✅ Full | **Complete** | ✅ Implicit |

**Workflow Events:**
- ✅ WorkflowStarted
- ✅ WorkflowCompleted
- ✅ WorkflowFailed
- ✅ WorkflowCancelled
- ✅ WorkflowSuspended
- ✅ WorkflowResumed

**Task Events:**
- ✅ TaskCreated - Tested in [tests/task_event_tests.rs](tests/task_event_tests.rs)
- ✅ TaskStarted
- ✅ TaskCompleted
- ✅ TaskRetried - Tested
- ✅ TaskFaulted
- ✅ TaskCancelled
- ✅ TaskSuspended
- ✅ TaskResumed

---

### 10.4 Visualization

| Feature | Implementation | Maturity | Test Coverage |
|---------|----------------|----------|---------------|
| **Graphviz Diagrams** | ✅ Full | **Complete** | ⚠️ Manual |
| **D2 Diagrams** | ✅ Full | **Complete** | ⚠️ Manual |
| **Workflow Graph Structure** | ✅ Full | **Complete** | ✅ Validation Tests |

---

### 10.5 Validation

| Feature | Implementation | Maturity | Test Coverage |
|---------|-------------|----------|---------------|
| **Schema Validation** | ✅ Full | **Complete** | ✅ 3 Tests |
| **Graph Structure Validation** | ✅ Full | **Complete** | ✅ Tests |
| **Workflow Definition Validation** | ✅ Full | **Complete** | ✅ Tests |

---

## 11. Test Coverage Summary

### 11.1 Test Infrastructure

| Test Type | Count | Location |
|-----------|-------|----------|
| **CTK Conformance Tests** | 20+ scenarios | [tests/ctk_conformance.rs](tests/ctk_conformance.rs) + [ctk/ctk/features/](ctk/ctk/features/) |
| **Integration Tests** | 30+ tests | [tests/](tests/) |
| **Unit Tests** | Embedded | Throughout source |

---

### 11.2 CTK Conformance Coverage

| Feature Category | CTK Feature File | Scenarios | Status |
|------------------|------------------|-----------|--------|
| **Call Task** | [call.feature](ctk/ctk/features/call.feature) | 5 | ✅ All Pass |
| **Data Flow** | [data-flow.feature](ctk/ctk/features/data-flow.feature) | 3 | ✅ All Pass |
| **Do Task** | [do.feature](ctk/ctk/features/do.feature) | 1 | ✅ Pass |
| **Emit Task** | [emit.feature](ctk/ctk/features/emit.feature) | 1 | ✅ Pass |
| **Flow Directive** | [flow.feature](ctk/ctk/features/flow.feature) | 2 | ✅ All Pass |
| **For Task** | [for.feature](ctk/ctk/features/for.feature) | 1 | ✅ Pass |
| **Raise Task** | [raise.feature](ctk/ctk/features/raise.feature) | 1 | ✅ Pass |
| **Set Task** | [set.feature](ctk/ctk/features/set.feature) | 1 | ✅ Pass |
| **Switch Task** | [switch.feature](ctk/ctk/features/switch.feature) | 3 | ✅ All Pass |
| **Try Task** | [try.feature](ctk/ctk/features/try.feature) | 2 | ✅ All Pass |
| **Fork Task** | [branch.feature](ctk/ctk/features/branch.feature) | 1 | ✅ Pass |

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