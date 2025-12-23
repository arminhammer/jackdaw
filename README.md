# jackdaw

`jackdaw` is a workflow execution engine for the [Serverless Workflows](https://serverlessworkflow.io/) specification. It supports durable execution of workflows through a persistence layer, as well as caching execution to prevent duplicate execution of expensive workflow tasks. `jackdaw` is written in Rust, and is designed for extensibiliy, performance, and easy to deploy into many contexts. 

The default version of `jackdaw` comes with a full embedded javascript interpreter (via [rustyscript](https://github.com/rscarson/rustyscript) and [deno_core](https://github.com/denoland/deno_core)), as well as embedded python (via [pyo3](https://pyo3.rs/)). This allows `jackdaw` to support Serverless Workflows that have python and javascript sections very efficiently.

`jackdaw` is also committed to always being free and useful open source software under the standard Apache 2.0 license. There's no risk of vendor lock-in, because any workflow you run with `jackdaw` can be executed by any of the other Serverless Workflow runtimes!

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

The `jackdaw` binary can be downloaded from the releases page. Please note that `jackdaw` requires `libpython` 3.14 to be installed.

#### jackdaw-lite

`jackdaw-lite` (statically-compiled without embedded python or javascript) can alternatively be downloaded. This version has no dependencies and is therefore very portable, but has more limited support for executing python and javascript. If you don't need support for those languages, this can be a very good option because it is self-contained and has no dependencies.

### From source

#### default version

The most straightforward way to install `jackdaw` is to clone the repository and run 

```bash
just build-release
```

This will compile the release binary

#### jackdaw-lite

`jackdaw-lite`, the statically-compiled version of `jackdaw` that does not have embedded javascript and python interpreters, can be built with 

```
just build-lite-static
```

## Usage

### `run`

#### container

`jackdaw` supports executing commands in containers. The default (and currently only) container runtime supported is Docker.

TODO: fix container output

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

```
jackdaw run ../../tests/fixtures/containers/container-env-vars.sw.yaml
```
![Run Container](docs/vhs/run-container.gif)

#### Python

#### Javascript

#### Tasks from a Catalog

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

![Cache Debug](docs/vhs/cache-debug.gif)

#### Persistence

### `validate`

```
jackdaw validate hello-world.sw.yaml
```

![Validate Command](docs/vhs/hello-world-validate.gif)

## Providers

### Cache Providers

#### in-memory

```bash
jackdaw run cache.sw.yaml --cache-provider memory -i '{ "userData": "user-data-1"}'
```

#### redb

```bash
jackdaw run cache.sw.yaml --cache-provider redb -i '{ "userData": "user-data-1"}'
```

#### sqlite

```bash
jackdaw run cache.sw.yaml --cache-provider sqlite --sqlite-db-url=cache.sqlite -i '{ "userData": "user-data-1"}'
```

#### postgres

```bash
jackdaw run cache.sw.yaml --cache-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "userData": "user-data-1"}'
```

### Persistence Providers

#### in-memory

```bash
jackdaw run persistence.sw.yaml --persistence-provider memory -i '{ "userData": "user-data-1"}'
```

#### redb

```bash
jackdaw run persistence.sw.yaml --persistence-provider redb -i '{ "userData": "user-data-1"}'
```

#### sqlite

```bash
jackdaw run persistence.sw.yaml --persistence-provider sqlite --sqlite-db-url=persistence.sqlite -i '{ "userData": "user-data-1"}'
```

#### postgres

```bash
jackdaw run persistence.sw.yaml --persistence-provider postgres --postgres-db-name=default --postgres-user default_user --postgres-password password --postgres-hostname localhost -i '{ "userData": "user-data-1"}'
```
### Container Providers

#### Docker

### Executor Providers

#### OpenAPI

#### Python

#### Python External

#### Typescript

#### Typescript External

#### OpenAPI

#### Rest

### Visualization

#### D2

#### Graphviz

## Supported Serverless Features Matrix

## Roadmap
