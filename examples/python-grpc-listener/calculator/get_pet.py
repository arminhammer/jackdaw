"""
Handler for GetPet RPC - fetches pet data from petstore API.
Demonstrates using third-party dependencies (requests library) in jackdaw gRPC listeners.
"""
import requests
from calculator.types import GetPetRequest, GetPetResponse


def handler(request: GetPetRequest) -> GetPetResponse:
    """
    Fetch pet information from the petstore API.

    Args:
        request: GetPetRequest containing pet_id

    Returns:
        GetPetResponse with pet information from the petstore API
    """
    pet_id = request["pet_id"]

    # Call the petstore API
    # Using https://petstore3.swagger.io which is commonly used in tests
    url = f"https://petstore3.swagger.io/api/v3/pet/{pet_id}"

    try:
        response = requests.get(url, timeout=10)
        response.raise_for_status()

        # Parse the JSON response from petstore
        pet_data = response.json()

        # Return the pet data - jackdaw will validate it against GetPetResponse schema
        result: GetPetResponse = {
            "id": pet_data.get("id", 0),
            "name": pet_data.get("name", "Unknown"),
            "status": pet_data.get("status", "unknown"),
        }

        return result

    except requests.exceptions.HTTPError as e:
        if e.response.status_code == 404:
            raise ValueError(f"Pet {pet_id} not found") from e
        raise ValueError(f"Failed to fetch pet: {e}") from e
    except requests.exceptions.RequestException as e:
        raise ValueError(f"Network error fetching pet: {e}") from e
