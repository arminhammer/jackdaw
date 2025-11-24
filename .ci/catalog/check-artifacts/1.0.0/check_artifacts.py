#!/usr/bin/env python3
"""
Check build artifacts for validity.
Strongly typed implementation.
"""
import os
import json
from typing import TypedDict


class CheckArtifactsInput(TypedDict):
    """Input schema for check_artifacts function."""
    binary_path: str


class CheckArtifactsOutput(TypedDict):
    """Output schema for check_artifacts function."""
    exists: bool
    executable: bool
    size_bytes: int
    path: str
    valid: bool


def check_artifacts(input_data: CheckArtifactsInput) -> CheckArtifactsOutput:
    """
    Verify that a binary exists and is executable.

    Args:
        input_data: Dictionary containing binary_path

    Returns:
        Dictionary with artifact validation results
    """
    binary_path = input_data['binary_path']

    result: CheckArtifactsOutput = {
        'exists': os.path.exists(binary_path),
        'executable': False,
        'size_bytes': 0,
        'path': binary_path,
        'valid': False
    }

    if result['exists']:
        try:
            stat = os.stat(binary_path)
            result['size_bytes'] = stat.st_size
            result['executable'] = os.access(binary_path, os.X_OK)
        except (IOError, OSError):
            pass

    result['valid'] = result['exists'] and result['executable']

    return result


# Main execution
if __name__ == '__main__':
    # Create typed input from arguments passed by workflow
    input_data: CheckArtifactsInput = {
        'binary_path': binary_path
    }

    # Execute function
    output = check_artifacts(input_data)

    # Print JSON output for workflow consumption
    print(json.dumps(output, indent=2))
