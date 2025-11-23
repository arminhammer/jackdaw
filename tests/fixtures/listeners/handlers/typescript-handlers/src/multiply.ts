/**
 * TypeScript handler for Calculator.Multiply operation.
 * Strongly typed: takes MultiplyRequest returns MultiplyResponse
 */

import type { MultiplyRequest, MultiplyResponse } from "./types.ts";

/**
 * Multiply two numbers.
 *
 * @param request - MultiplyRequest with 'a' and 'b' as int32 values
 * @returns MultiplyResponse with 'result' field containing the product
 */
export function handler(request: MultiplyRequest): MultiplyResponse {
  // Perform calculation using strongly-typed inputs
  const result: number = request.a * request.b;

  // Create and return strongly-typed response
  const response: MultiplyResponse = { result };
  return response;
}
