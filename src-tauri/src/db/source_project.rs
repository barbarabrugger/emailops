//! Storage reads for the offline source exporter. Existing sync does not capture MIME.
//! The account is checked before reading any body/blob, within one read transaction.
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::Email;
use rusqlite::{params, OptionalExtension};

pub(crate) struct OriginalSource {
    pub bytes: Vec<u8>,
    pub provenance: String,
    pub captured_at: i64,
}

pub(crate) struct SourceRecord {
    pub email: Email,
    pub body: Option<String>,
    pub provider_payload: Option<String>,
    pub original: Option<OriginalSource>,
    pub raw_capture_supported: bool,
}

impl Database {
    pub(crate) fn source_record(&self, account_id: &str, email_id: &str) -> Result<SourceRecord> {
        let conn = self.reader();
        let tx = conn.unchecked_transaction()?;
        let sql = format!(
            "SELECT {} FROM emails WHERE account_id = ?1 AND id = ?2",
            super::emails::EMAIL_COLUMNS
        );
        let email = tx
            .query_row(&sql, params![account_id, email_id], super::emails::row_to_email)
            .optional()?
            .ok_or_else(|| AppError::NotFound("email not found in selected account".into()))?;
        let body = tx
            .query_row("SELECT body FROM email_bodies WHERE email_id = ?1", [email_id], |row| {
                row.get(0)
            })
            .optional()?;
        let provider_payload = tx.query_row(
            "SELECT raw_json FROM emails WHERE account_id = ?1 AND id = ?2",
            params![account_id, email_id],
            |row| row.get(0),
        )?;
        // Older databases remain readable without a migrating/writable open.
        let raw_capture_supported: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'original_email_sources')",
            [],
            |row| row.get(0),
        )?;
        let original = if raw_capture_supported {
            tx.query_row(
                "SELECT raw_mime, provenance, captured_at FROM original_email_sources WHERE email_id = ?1",
                [email_id],
                |row| {
                    Ok(OriginalSource {
                        bytes: row.get(0)?,
                        provenance: row.get(1)?,
                        captured_at: row.get(2)?,
                    })
                },
            )
            .optional()?
        } else {
            None
        };
        tx.commit()?;
        Ok(SourceRecord {
            email,
            body,
            provider_payload,
            original,
            raw_capture_supported,
        })
    }

    /// Only called after the prototype seed has reserved a new empty database.
    pub(crate) fn insert_synthetic_source_project(&self, body: &str, raw: &[u8]) -> Result<()> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO accounts (id, provider, email, name, created_at, enabled)
             VALUES ('synthetic', 'imap', 'owner@example.invalid', 'Synthetic', 1800000000, 0)",
            [],
        )?;
        tx.execute(
            "INSERT INTO emails (id, account_id, thread_id, message_id, subject, sender, sender_email,
                recipients_json, snippet, timestamp, created_at)
             VALUES ('room-mail', 'synthetic', 'room-thread', '<room@example.invalid>', 'Room Renovation',
                'Builder', 'builder@example.invalid', '[\"owner@example.invalid\"]', 'Confirm materials',
                1800000000, 1800000000)",
            [],
        )?;
        tx.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES ('room-mail', ?1)",
            [body],
        )?;
        tx.execute(
            "INSERT INTO original_email_sources (email_id, raw_mime, provenance, captured_at)
             VALUES ('room-mail', ?1, 'synthetic fixture; no provider connection', 1800000000)",
            params![raw],
        )?;
        tx.execute(
            "INSERT INTO pending_tasks (id, account_id, title, detail, source, source_email_id,
                source_thread_id, assignee, status, priority, due_at, created_at, updated_at)
             VALUES ('room-task', 'synthetic', 'Confirm materials', 'Review both attached material lists',
                'user', 'room-mail', 'room-thread', 'me', 'open', 'normal',
                1800086400, 1800000000, 1800000000)",
            [],
        )?;
        tx.execute(
            "INSERT INTO calendar_events (id, account_id, provider_event_id, title, description,
                start_time, end_time, timezone, created_at, updated_at)
             VALUES ('room-event', 'synthetic', 'synthetic-site-visit', 'Site visit',
                'Room Renovation planning visit', 1800172800, 1800176400, 'UTC',
                1800000000, 1800000000)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}
