/**
 * JavaScript handler for Calculator.Add operation.
 * Uses JSDoc for type documentation.
 *
 * @param {Object} request - AddRequest with 'a' and 'b' as int32 values
 * @param {number} request.a - First operand
 * @param {number} request.b - Second operand
 * @returns {Object} AddResponse with 'result' field containing the sum
 */
export function handler(request) {
  // Perform calculation
  const result = request.a + request.b;

  // Create and return response
  const response = { result };
  return response;
}
