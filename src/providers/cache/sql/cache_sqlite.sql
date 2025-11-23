-- SQLite Cache Schema
-- Stores cached task execution results for idempotent task execution

CREATE TABLE IF NOT EXISTS cache_entries (
    key TEXT PRIMARY KEY NOT NULL,
    inputs TEXT NOT NULL,           -- JSON serialized
    output TEXT NOT NULL,            -- JSON serialized
    timestamp DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Index for potential TTL-based cleanup or timestamp queries
CREATE INDEX IF NOT EXISTS idx_cache_timestamp ON cache_entries(timestamp);
