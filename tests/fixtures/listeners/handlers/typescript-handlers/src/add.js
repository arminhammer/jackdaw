/**
 * JavaScript handler for Calculator.Add operation.
 * Takes AddRequest and returns AddResponse
 */

/**
 * Add two numbers.
 *
 * @param {Object} request - AddRequest with 'a' and 'b' as int32 values
 * @param {number} request.a - First number
 * @param {number} request.b - Second number
 * @returns {Object} AddResponse with 'result' field containing the sum
 */
export function handler(request) {
  // Perform calculation
  const result = request.a + request.b;

  // Create and return response
  const response = { result };
  return response;
}
