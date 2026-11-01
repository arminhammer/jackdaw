-- PostgreSQL Cache Schema
-- Stores cached task execution results for idempotent task execution
-- Uses JSONB for better performance on PostgreSQL

CREATE TABLE IF NOT EXISTS cache_entries (
    key TEXT PRIMARY KEY NOT NULL,
    inputs JSONB NOT NULL,
    output JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for potential TTL-based cleanup or timestamp queries
CREATE INDEX IF NOT EXISTS idx_cache_timestamp ON cache_entries(timestamp);

-- GIN index for efficient JSONB queries on inputs (optional, for future enhancements)
CREATE INDEX IF NOT EXISTS idx_cache_inputs ON cache_entries USING GIN (inputs);
