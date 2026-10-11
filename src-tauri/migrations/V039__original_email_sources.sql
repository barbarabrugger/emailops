-- Optional original MIME capture. Existing sync does not populate this table.
-- Account scope is derived through the email foreign key, never trusted from callers.
CREATE TABLE IF NOT EXISTS original_email_sources (
    email_id TEXT PRIMARY KEY,
    raw_mime BLOB NOT NULL,
    provenance TEXT NOT NULL,
    captured_at INTEGER NOT NULL,
    FOREIGN KEY (email_id) REFERENCES emails(id) ON DELETE CASCADE
);
