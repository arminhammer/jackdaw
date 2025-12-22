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

### run


### validate

```
jackdaw validate hello-world.sw.yaml
```

![Validate Command](docs/vhs/hello-world-validate.gif)