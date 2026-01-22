#!/usr/bin/env python3
"""
Simple workflow execution example using jackdaw Python bindings.

Build the Python module first:
    maturin develop --features python

Run this example:
    python examples/python/simple_workflow.py
"""

import asyncio
import jackdaw

WORKFLOW_YAML = """
document:
  dsl: '1.0.2'
  namespace: examples
  name: simple-transform
  version: 1.0.0
do:
  - greet:
      set:
        greeting: "Hello, World!"
        user: "${ .user }"
  - transform:
      set:
        message: "${ \\"Greeting: \\" + .greeting }"
        full_message: "${ .message + \\" User: \\" + .user }"
"""

async def main():
    builder = jackdaw.DurableEngineBuilder()
    engine = builder.build()

    input_data = {"user": "Alice"}

    handle = await engine.execute(WORKFLOW_YAML, input_data)

    print(f"Workflow instance ID: {handle.instance_id()}")

    result = await handle.wait_for_completion(30.0)

    print(f"Result: {result}")

if __name__ == "__main__":
    asyncio.run(main())
