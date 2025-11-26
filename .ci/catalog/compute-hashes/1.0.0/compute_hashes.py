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
# Arguments are injected as global variables by the workflow runtime
if __name__ == '__main__':
    # Build typed input from injected global 'categories' variable
    input_data: ComputeHashesInput = {
        'categories': categories  # type: ignore - injected by workflow runtime
    }

    # Execute strongly-typed handler
    output = handler(input_data)

    # Output result as JSON for workflow consumption
    print(json.dumps(output))
