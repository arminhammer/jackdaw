"""
Get current timestamp for cache invalidation.
Strongly typed implementation with Serverless Workflow spec compliance.
"""
import json
from datetime import datetime
from typing import TypedDict


class GetTimestampOutput(TypedDict):
    """Output schema for handler function."""
    timestamp: str


def handler() -> GetTimestampOutput:
    """
    Get current timestamp in ISO format.

    Returns:
        Dictionary with timestamp field
    """
    return GetTimestampOutput(timestamp=datetime.now().isoformat())


# Serverless Workflow script execution
if __name__ == '__main__':
    # Execute strongly-typed handler
    output = handler()

    # Output result as JSON for workflow consumption
    print(json.dumps(output))
