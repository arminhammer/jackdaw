-- PostgreSQL Persistence Schema
-- Event sourcing and checkpoint storage for durable workflow execution
-- Uses JSONB for better performance on PostgreSQL

-- Workflow Events Table: Stores all workflow events for replay and audit
CREATE TABLE IF NOT EXISTS workflow_events (
    id BIGSERIAL PRIMARY KEY,
    instance_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL,
    sequence_number BIGINT NOT NULL
);

-- Indexes for efficient event queries
CREATE INDEX IF NOT EXISTS idx_events_instance_id ON workflow_events(instance_id);
CREATE INDEX IF NOT EXISTS idx_events_instance_seq ON workflow_events(instance_id, sequence_number);

-- GIN index for efficient JSONB queries on event data (optional, for future enhancements)
CREATE INDEX IF NOT EXISTS idx_events_data ON workflow_events USING GIN (event_data);

-- Workflow Checkpoints Table: Stores latest state per workflow instance
CREATE TABLE IF NOT EXISTS workflow_checkpoints (
    instance_id TEXT PRIMARY KEY NOT NULL,
    current_task TEXT NOT NULL,
    data JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL
);
