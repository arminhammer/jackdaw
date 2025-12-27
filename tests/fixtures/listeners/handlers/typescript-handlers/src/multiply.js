/**
 * JavaScript handler for Calculator.Multiply operation.
 * Takes MultiplyRequest and returns MultiplyResponse
 */

/**
 * Multiply two numbers.
 *
 * @param {Object} request - MultiplyRequest with 'a' and 'b' as int32 values
 * @param {number} request.a - First number
 * @param {number} request.b - Second number
 * @returns {Object} MultiplyResponse with 'result' field containing the product
 */
export function handler(request) {
  // Perform calculation
  const result = request.a * request.b;

  // Create and return response
  const response = { result };
  return response;
}
