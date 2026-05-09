"""Build and run a two-step workflow using WorkflowBuilder.

Module-level imports (urllib, json, os) are detected automatically and
prepended to each embedded step — no need to put imports inside the function.
"""

import json
import os
import urllib.request

import jackdaw


def fetch_data(working_dir: str, source_url: str) -> dict:
    os.makedirs(working_dir, exist_ok=True)
    out_path = os.path.join(working_dir, "data.json")
    with urllib.request.urlopen(source_url) as r:
        data = json.loads(r.read())
    with open(out_path, "w") as f:
        json.dump(data, f)
    return {"data_file": out_path}


def summarize(data_file: str) -> dict:
    with open(data_file) as f:
        data = json.load(f)
    return {"record_count": len(data) if isinstance(data, list) else 1}


wf = (
    jackdaw.WorkflowBuilder("fetch-and-summarize", namespace="examples")
    .run_python("fetch", fetch_data)
    .run_python("summarize", summarize)
    .build()
)

if __name__ == "__main__":
    result = jackdaw.run(
        wf,
        input={
            "working_dir": "/tmp/example",
            "source_url": "https://jsonplaceholder.typicode.com/todos",
        },
    )
    print(result)
