/**
 * JavaScript handler for GetPet operation - fetches pet data from petstore API.
 * Demonstrates using third-party libraries via CDN in jackdaw listeners.
 *
 * @param {Object} request - Request containing petId
 * @param {number} request.petId - ID of pet to fetch
 * @returns {Promise<Object>} Pet information from the petstore API
 */
// Using wretch - a tiny wrapper around fetch that works in Deno without extra globals
// This demonstrates using third-party libraries via CDN imports
import wretch from "https://esm.sh/wretch@2.9.0";

export async function handler(request) {
  const petId = request.petId;

  if (!petId) {
    throw new Error("petId is required");
  }

  // Call the petstore API using wretch (third-party HTTP client from CDN)
  // Using https://petstore3.swagger.io which is commonly used in tests
  const url = `https://petstore3.swagger.io/api/v3/pet/${petId}`;

  try {
    // wretch automatically parses JSON responses
    const petData = await wretch(url).get().json();

    // Return the pet data - jackdaw will validate it against the OpenAPI schema
    const result = {
      id: petData.id || 0,
      name: petData.name || "Unknown",
      status: petData.status || "unknown",
    };

    return result;
  } catch (error) {
    // Check for 404 errors
    if (error.status === 404) {
      throw new Error(`Pet ${petId} not found`);
    }
    if (error.status) {
      throw new Error(`Failed to fetch pet: HTTP ${error.status}`);
    }
    // Network or other errors
    throw new Error(`Network error fetching pet: ${error.message}`);
  }
}
