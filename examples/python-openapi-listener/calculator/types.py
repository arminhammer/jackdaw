"""
Type definitions for Calculator service.
These types match the protobuf and OpenAPI specifications exactly.
"""
from typing import TypedDict


class AddRequest(TypedDict):
    """Request type for Add operation matching proto AddRequest"""
    a: int
    b: int


class AddResponse(TypedDict):
    """Response type for Add operation matching proto AddResponse"""
    result: int


class MultiplyRequest(TypedDict):
    """Request type for Multiply operation matching proto MultiplyRequest"""
    a: int
    b: int


class MultiplyResponse(TypedDict):
    """Response type for Multiply operation matching proto MultiplyResponse"""
    result: int


class PetResponse(TypedDict, total=False):
    """Response type for getPet operation - fetches pet from petstore API"""
    id: int
    name: str
    status: str
    category: dict
    photoUrls: list[str]
    tags: list[dict]
