/**
 * JavaScript handler for GetPet operation - fetches pet data from petstore API.
 * Demonstrates using npm dependencies in jackdaw listeners.
 *
 * @param {Object} request - Request containing petId
 * @param {number} request.petId - ID of pet to fetch
 * @returns {Promise<Object>} Pet information from the petstore API
 */
// Using axios - a popular HTTP client from npm
// This demonstrates using npm dependencies with Node.js
// Install dependencies with: npm install
import axios from "axios";

export async function handler(request) {
  const petId = request.petId;

  if (!petId) {
    throw new Error("petId is required");
  }

  // Call the petstore API using axios (from npm)
  // Using https://petstore3.swagger.io which is commonly used in tests
  const url = `https://petstore3.swagger.io/api/v3/pet/${petId}`;

  try {
    // axios automatically parses JSON responses
    const response = await axios.get(url);
    const petData = response.data;

    // Return the pet data - jackdaw will validate it against the OpenAPI schema
    const result = {
      id: petData.id || 0,
      name: petData.name || "Unknown",
      status: petData.status || "unknown",
    };

    return result;
  } catch (error) {
    // Check for 404 errors
    if (error.response?.status === 404) {
      throw new Error(`Pet ${petId} not found`);
    }
    if (error.response?.status) {
      throw new Error(`Failed to fetch pet: HTTP ${error.response.status}`);
    }
    // Network or other errors
    throw new Error(`Network error fetching pet: ${error.message}`);
  }
}
