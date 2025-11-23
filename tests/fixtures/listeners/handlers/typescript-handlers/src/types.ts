/**
 * Type definitions for Calculator service.
 * These types match the protobuf and OpenAPI specifications exactly.
 */

/**
 * Request type for Add operation matching proto AddRequest
 */
export interface AddRequest {
  a: number; // int32
  b: number; // int32
}

/**
 * Response type for Add operation matching proto AddResponse
 */
export interface AddResponse {
  result: number; // int32
}

/**
 * Request type for Multiply operation matching proto MultiplyRequest
 */
export interface MultiplyRequest {
  a: number; // int32
  b: number; // int32
}

/**
 * Response type for Multiply operation matching proto MultiplyResponse
 */
export interface MultiplyResponse {
  result: number; // int32
}
