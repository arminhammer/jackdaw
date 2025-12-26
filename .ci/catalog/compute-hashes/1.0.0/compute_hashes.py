"""
Compute file hashes for CI cache invalidation.
Strongly typed implementation with Serverless Workflow spec compliance.
"""
import hashlib
import glob
import json
from typing import Dict, List, TypedDict


class CategoryConfig(TypedDict):
    """Configuration for a file category to hash."""
    patterns: List[str]


class ComputeHashesInput(TypedDict, total=False):
    """Input schema for handler function."""
    categories: Dict[str, CategoryConfig]
    workflow_id: str  # Workflow execution ID for cache busting


def handler(request: ComputeHashesInput) -> Dict[str, str]:
    """
    Compute SHA256 hashes for different file categories.

    Args:
        request: Dictionary containing category configurations

    Returns:
        Dictionary mapping category names to their hash results
    """
    results: Dict[str, str] = {}

    for category_name, config in request['categories'].items():
        files: List[str] = []

        # Collect all files matching the patterns
        for pattern in config['patterns']:
            matched = glob.glob(pattern, recursive=True)
            files.extend(matched)

        # Sort for deterministic ordering
        files.sort()

        # Compute combined hash
        hasher = hashlib.sha256()
        for file_path in files:
            try:
                with open(file_path, 'rb') as f:
                    file_content = f.read()
                    hasher.update(file_content)
            except (IOError, OSError):
                # Skip files that can't be read
                pass

        results[category_name] = hasher.hexdigest()

    return results


# Serverless Workflow script execution
# Arguments are passed via sys.argv (after script name at index 0)
if __name__ == '__main__':
    import sys

    # Parse arguments from sys.argv
    # sys.argv[0] is script name, sys.argv[1] is first arg, etc.
    if len(sys.argv) < 3:
        print(f"Error: Expected 2 arguments, got {len(sys.argv) - 1}", file=sys.stderr)
        print(f"sys.argv: {sys.argv}", file=sys.stderr)
        sys.exit(1)

    # Arguments are JSON-serialized when passed through sys.argv
    categories = json.loads(sys.argv[1])
    workflow_id = sys.argv[2]

    print(f"Computing hashes for workflow ID: {workflow_id}", file=sys.stderr)
    print(f"Categories: {json.dumps(categories)}", file=sys.stderr)

    # Build typed input
    input_data: ComputeHashesInput = {
        'categories': categories,
        'workflow_id': workflow_id
    }

    # Execute strongly-typed handler
    output = handler(input_data)

    # Log all hashes to stderr for debugging
    print(json.dumps(output), file=sys.stderr)

    # Return structured output with hash in stdout field
    result = {
        "stdout": output.get("all_sources", ""),
        "hashes": output
    }
    print(json.dumps(result))
