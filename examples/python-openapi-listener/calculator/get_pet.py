"""
Handler for getPet operation - fetches pet data from petstore API.
Demonstrates using third-party dependencies (requests library) in jackdaw listeners.
"""
import requests
from calculator.types import PetResponse


def handler(event: dict) -> PetResponse:
    """
    Fetch pet information from the petstore API.

    The event contains path parameters from the OpenAPI spec.
    For GET /pet/{petId}, the petId is available in the event.

    Args:
        event: CloudEvent envelope containing the request data

    Returns:
        PetResponse with pet information from the petstore API
    """
    # Extract petId from the event
    # The listener wraps the request in a CloudEvent envelope
    # Path parameters are available in the event data
    pet_id = event.get("petId")

    if not pet_id:
        raise ValueError("petId is required")

    # Call the petstore API
    # Using https://petstore3.swagger.io which is commonly used in CTK tests
    url = f"https://petstore3.swagger.io/api/v3/pet/{pet_id}"

    try:
        response = requests.get(url, timeout=10)
        response.raise_for_status()

        # Parse the JSON response from petstore
        pet_data = response.json()

        # Return the pet data - jackdaw will validate it against PetResponse schema
        result: PetResponse = {
            "id": pet_data.get("id", 0),
            "name": pet_data.get("name", "Unknown"),
            "status": pet_data.get("status", "unknown"),
        }

        # Add optional fields if present
        if "category" in pet_data:
            result["category"] = pet_data["category"]
        if "photoUrls" in pet_data:
            result["photoUrls"] = pet_data["photoUrls"]
        if "tags" in pet_data:
            result["tags"] = pet_data["tags"]

        return result

    except requests.exceptions.HTTPError as e:
        if e.response.status_code == 404:
            raise ValueError(f"Pet {pet_id} not found") from e
        raise ValueError(f"Failed to fetch pet: {e}") from e
    except requests.exceptions.RequestException as e:
        raise ValueError(f"Network error fetching pet: {e}") from e