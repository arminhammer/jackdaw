from calculator.types import MultiplyRequest, MultiplyResponse


def handler(request: MultiplyRequest) -> MultiplyResponse:
    """
    Multiply two numbers.

    Args:
        request: MultiplyRequest with 'a' and 'b' as int32 values

    Returns:
        MultiplyResponse with 'result' field containing the product
    """
    # Perform calculation using strongly-typed inputs
    result: int = request["a"] * request["b"]

    # Create and return strongly-typed response
    response: MultiplyResponse = {"result": result}
    return response
