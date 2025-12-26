"""
Python handler for Calculator.Add operation.
Strongly typed: takes AddRequest returns AddResponse
"""
from calculator.types import AddRequest, AddResponse


def handler(request: AddRequest) -> AddResponse:
    """
    Add two numbers.

    Args:
        request: AddRequest with 'a' and 'b' as int32 values

    Returns:
        AddResponse with 'result' field containing the sum
    """
    # Perform calculation using strongly-typed inputs
    result: int = request["a"] + request["b"]

    # Create and return strongly-typed response
    response: AddResponse = {"result": result}
    return response
