#!/usr/bin/env python3
"""
Event streaming example showing workflow execution with custom database.

Build the Python module first:
    maturin develop --features python

Run this example:
    python examples/python/event_streaming.py
"""

import asyncio
import jackdaw

WORKFLOW_YAML = """
document:
  dsl: '1.0.2'
  namespace: examples
  name: multi-step-process
  version: 1.0.0
do:
  - initialize:
      set:
        status: "started"
        input_value: "${ .value }"
  - process:
      set:
        doubled: "${ (.input_value | tonumber) * 2 }"
        status: "processing"
  - finalize:
      set:
        result: "${ (.doubled | tonumber) + 10 }"
        status: "completed"
"""

async def main():
    builder = jackdaw.DurableEngineBuilder()
    engine = builder.build()

    input_data = {"value": 5}

    handle = await engine.execute(WORKFLOW_YAML, input_data)

    print(f"Workflow instance ID: {handle.instance_id()}")

    result = await handle.wait_for_completion(30.0)

    print(f"Final result: {result}")

if __name__ == "__main__":
    asyncio.run(main())
