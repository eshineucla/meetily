-- Migration: Keep every generated summary as a numbered version, and let a
-- meeting's minutes be marked final (read-only)

CREATE TABLE IF NOT EXISTS summary_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    -- 'generated' by the model, or 'edited' when a hand-edited summary was kept before a restore
    source TEXT NOT NULL,
    result TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    UNIQUE (meeting_id, version)
);

CREATE INDEX IF NOT EXISTS idx_summary_versions_meeting ON summary_versions(meeting_id);

-- Existing summaries become version 1
INSERT INTO summary_versions (meeting_id, version, created_at, source, result)
SELECT meeting_id, 1, COALESCE(end_time, updated_at), 'generated', result
FROM summary_processes
WHERE result IS NOT NULL AND LOWER(status) = 'completed';

ALTER TABLE meetings ADD COLUMN finalized_at TEXT;
