-- SQLite Persistence Schema
-- Event sourcing and checkpoint storage for durable workflow execution

-- Workflow Events Table: Stores all workflow events for replay and audit
CREATE TABLE IF NOT EXISTS workflow_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_data TEXT NOT NULL,           -- JSON serialized WorkflowEvent
    timestamp DATETIME NOT NULL,
    sequence_number INTEGER NOT NULL
);

-- Indexes for efficient event queries
CREATE INDEX IF NOT EXISTS idx_events_instance_id ON workflow_events(instance_id);
CREATE INDEX IF NOT EXISTS idx_events_instance_seq ON workflow_events(instance_id, sequence_number);

-- Workflow Checkpoints Table: Stores latest state per workflow instance
CREATE TABLE IF NOT EXISTS workflow_checkpoints (
    instance_id TEXT PRIMARY KEY NOT NULL,
    current_task TEXT NOT NULL,
    data TEXT NOT NULL,                 -- JSON serialized
    timestamp DATETIME NOT NULL
);
