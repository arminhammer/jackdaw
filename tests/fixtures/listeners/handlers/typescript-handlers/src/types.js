/**
 * Type definitions for Calculator service.
 * These types match the protobuf and OpenAPI specifications exactly.
 * Using JSDoc notation for JavaScript (no TypeScript syntax)
 */

/**
 * Request type for Add operation matching proto AddRequest
 * @typedef {Object} AddRequest
 * @property {number} a - First number (int32)
 * @property {number} b - Second number (int32)
 */

/**
 * Response type for Add operation matching proto AddResponse
 * @typedef {Object} AddResponse
 * @property {number} result - Sum result (int32)
 */

/**
 * Request type for Multiply operation matching proto MultiplyRequest
 * @typedef {Object} MultiplyRequest
 * @property {number} a - First number (int32)
 * @property {number} b - Second number (int32)
 */

/**
 * Response type for Multiply operation matching proto MultiplyResponse
 * @typedef {Object} MultiplyResponse
 * @property {number} result - Product result (int32)
 */

// Export empty object to make this a valid ES module
export {};
