/**
 * TypeScript handler for Calculator.Add operation.
 * Strongly typed: takes AddRequest returns AddResponse
 */

import type { AddRequest, AddResponse } from "./types.ts";

/**
 * Add two numbers.
 *
 * @param request - AddRequest with 'a' and 'b' as int32 values
 * @returns AddResponse with 'result' field containing the sum
 */
export function handler(request: AddRequest): AddResponse {
  // Perform calculation using strongly-typed inputs
  const result: number = request.a + request.b;

  // Create and return strongly-typed response
  const response: AddResponse = { result };
  return response;
}
