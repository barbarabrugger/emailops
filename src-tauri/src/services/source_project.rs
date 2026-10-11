//! Deterministic source export and dossiers referencing existing records.
use crate::db::Database;
use crate::models::error::{AppError, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use mailparse::{MailHeaderMap, ParsedMail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub account_id: String,
    pub email_ids: Vec<String>,
    pub task_ids: Vec<String>,
    pub event_ids: Vec<String>,
}

/// Read one scoped record, then build the export without providers, models or file paths.
pub fn source(db: &Database, account_id: &str, email_id: &str) -> Result<Value> {
    let record = db.source_record(account_id, email_id)?;
    let mut parts = Vec::new();
    let mut issues = Vec::new();
    let mut parsed = false;
    let raw_mime = match &record.original {
        Some(original) => {
            if original.bytes.is_empty() {
                issues.push("empty_original_mime".into());
            } else {
                match mailparse::parse_mail(&original.bytes) {
                    Ok(mail) => {
                        parsed = true;
                        collect_parts(&mail, "1", None, 0, &mut parts, &mut issues);
                    }
                    Err(error) => issues.push(format!("mime_parse_failed: {error}")),
                }
            }
            json!({
                "available": true,
                "base64": STANDARD.encode(&original.bytes),
                "sha256": sha256(&original.bytes),
                "byte_length": original.bytes.len(),
                "provenance": original.provenance,
                "captured_at": original.captured_at
            })
        }
        None => {
            issues.push(if record.raw_capture_supported {
                "original_mime_not_captured".into()
            } else {
                "raw_capture_schema_unavailable".into()
            });
            json!({"available": false, "base64": null, "sha256": null,
                   "byte_length": null, "provenance": null, "captured_at": null})
        }
    };
    let mut value = json!({
        "schema_version": 1,
        "email_id": record.email.id,
        "account_id": record.email.account_id,
        "thread_id": record.email.thread_id,
        "message_id": record.email.message_id,
        "subject": record.email.subject,
        "sender": record.email.sender,
        "sender_email": record.email.sender_email,
        "recipients": record.email.recipients,
        "cc": record.email.cc,
        "timestamp": record.email.timestamp,
        "cached_body": cached_text(record.body.as_deref(), "email_bodies.body"),
        "provider_payload": cached_text(record.provider_payload.as_deref(), "emails.raw_json"),
        "raw_mime": raw_mime,
        "mime_parts": parts,
        "completeness": {
            "original_source_complete": record.original.is_some() && parsed && issues.is_empty(),
            "missing_raw": record.original.is_none(),
            "missing_cached_body": record.body.is_none(),
            "raw_capture_supported": record.raw_capture_supported,
            "mime_parsed": parsed,
            "issues": issues
        }
    });
    add_hash(&mut value)?;
    Ok(value)
}

/// A dossier is an explicit projection of existing records, never a new task list.
pub fn project(db: &Database, account_id: &str, manifest: &ProjectManifest) -> Result<Value> {
    let manifest = normalized_manifest(account_id, manifest)?;
    if db.get_account(account_id)?.is_none() {
        return Err(AppError::NotFound("account not found".into()));
    }
    let mut sources = Vec::new();
    let mut tasks = Vec::new();
    let mut events = Vec::new();
    let mut missing = Vec::new();
    for id in &manifest.email_ids {
        match source(db, account_id, id) {
            Ok(value) => sources.push(value),
            Err(AppError::NotFound(_)) => missing.push(json!({"kind": "email", "id": id})),
            Err(error) => return Err(error),
        }
    }
    for id in &manifest.task_ids {
        match db.get_pending_task_by_id(account_id, id)? {
            Some(task) => tasks.push(task),
            None => missing.push(json!({"kind": "task", "id": id})),
        }
    }
    for id in &manifest.event_ids {
        match db.get_calendar_event_by_id(account_id, id)? {
            Some(event) => events.push(event),
            None => missing.push(json!({"kind": "event", "id": id})),
        }
    }
    let complete = missing.is_empty()
        && sources
            .iter()
            .all(|source| source["completeness"]["original_source_complete"] == true);
    let mut value = json!({
        "schema_version": 1,
        "manifest": manifest,
        "email_sources": sources,
        "tasks": tasks,
        "events": events,
        "missing_references": missing,
        "complete": complete
    });
    add_hash(&mut value)?;
    Ok(value)
}

/// Bootstrap only synthetic data. No default path, credentials, models or provider calls.
/// A create-new reservation ensures even a racing invocation cannot overwrite an existing DB.
pub fn seed(data_dir: &Path) -> Result<Value> {
    if data_dir.as_os_str().is_empty() {
        return Err(AppError::InvalidInput(
            "seed destination must be explicit and nonempty".into(),
        ));
    }
    match std::fs::symlink_metadata(data_dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(AppError::InvalidInput(
                    "seed destination must be an empty directory, not a link or file".into(),
                ));
            }
            if std::fs::read_dir(data_dir)
                .map_err(io_error)?
                .next()
                .transpose()
                .map_err(io_error)?
                .is_some()
            {
                return Err(AppError::InvalidInput("seed destination must be empty".into()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(data_dir).map_err(io_error)?;
        }
        Err(error) => return Err(io_error(error)),
    }
    let reservation = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(data_dir.join("emailops.db"))
        .map_err(io_error)?;
    drop(reservation);
    let db = Database::new(data_dir.to_path_buf())?;
    let body = format!(
        "{}END OF SYNTHETIC SOURCE\r\n",
        "Room Renovation: confirm materials ä 🛠.\r\n".repeat(800)
    );
    let raw = synthetic_mime(&body);
    db.insert_synthetic_source_project(&body, &raw)?;
    let manifest = ProjectManifest {
        schema_version: 1,
        id: "room-renovation".into(),
        title: "Room Renovation".into(),
        account_id: "synthetic".into(),
        email_ids: vec!["room-mail".into()],
        task_ids: vec!["room-task".into()],
        event_ids: vec!["room-event".into()],
    };
    let manifest_path = data_dir.join("project-manifest.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)
        .map_err(io_error)?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)
        .map_err(io_error)?;
    file.write_all(b"\n").map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    Ok(json!({"schema_version": 1, "account_id": "synthetic",
              "manifest_path": manifest_path, "email_ids": manifest.email_ids}))
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::IoError(error.to_string())
}

fn normalized_manifest(account_id: &str, input: &ProjectManifest) -> Result<ProjectManifest> {
    if input.schema_version != 1 || input.account_id != account_id {
        return Err(AppError::InvalidInput(
            "manifest version or selected account does not match".into(),
        ));
    }
    if input.id.trim().is_empty() || input.title.trim().is_empty() || account_id.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "manifest id, title and account must be nonempty".into(),
        ));
    }
    let mut manifest = input.clone();
    for ids in [&mut manifest.email_ids, &mut manifest.task_ids, &mut manifest.event_ids] {
        if ids.iter().any(|id| id.trim().is_empty()) {
            return Err(AppError::InvalidInput("manifest reference ids must be nonempty".into()));
        }
        ids.sort();
        ids.dedup();
    }
    Ok(manifest)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn cached_text(text: Option<&str>, provenance: &str) -> Value {
    json!({"available": text.is_some(), "text": text,
        "sha256": text.map(|text| sha256(text.as_bytes())),
        "provenance": provenance, "original_source": false})
}

/// Sort object keys recursively; array order remains meaningful. No generated clock fields.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: std::collections::BTreeMap<_, _> = object
                .iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        value => value.clone(),
    }
}

fn add_hash(value: &mut Value) -> Result<()> {
    let digest = sha256(&serde_json::to_vec(&canonical(value))?);
    value["content_sha256"] = Value::String(digest);
    Ok(())
}

/// Preserve every node, including containers, inline data and encapsulated messages.
/// IDs are positional within their source: root 1, children 1.1, 1.2, etc.
fn collect_parts(
    part: &ParsedMail<'_>,
    id: &str,
    parent_id: Option<&str>,
    depth: usize,
    parts: &mut Vec<Value>,
    issues: &mut Vec<String>,
) {
    let multipart = part.ctype.mimetype.to_ascii_lowercase().starts_with("multipart/");
    let disposition = part.get_content_disposition();
    let filename = disposition
        .params
        .get("filename")
        .or_else(|| part.ctype.params.get("name"));
    let decoded = if multipart {
        None
    } else {
        match part.get_body_raw() {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                issues.push(format!("part {id}: transfer_decode_failed: {error}"));
                None
            }
        }
    };
    let text = if part.ctype.mimetype.to_ascii_lowercase().starts_with("text/") && decoded.is_some() {
        match part.get_body() {
            Ok(text) => Some(text),
            Err(error) => {
                issues.push(format!("part {id}: text_decode_failed: {error}"));
                None
            }
        }
    } else {
        None
    };
    let headers: Vec<_> = part
        .headers
        .iter()
        .map(|header| json!({"name": header.get_key(), "value": header.get_value()}))
        .collect();
    parts.push(json!({
        "id": id, "parent_id": parent_id,
        "mime_type": part.ctype.mimetype, "filename": filename,
        "content_id": part.headers.get_first_value("Content-ID"),
        "disposition": part.headers.get_first_value("Content-Disposition")
            .map(|value| value.split(';').next().unwrap_or("").trim().to_ascii_lowercase()),
        "headers": headers,
        "raw_base64": STANDARD.encode(part.raw_bytes),
        "raw_sha256": sha256(part.raw_bytes),
        "raw_byte_length": part.raw_bytes.len(),
        "decoded_base64": decoded.as_deref().map(|bytes| STANDARD.encode(bytes)),
        "decoded_sha256": decoded.as_deref().map(sha256),
        "decoded_byte_length": decoded.as_deref().map(<[u8]>::len),
        "decoded_text": text
    }));
    if multipart {
        // mailparse accepts empty/truncated containers; parsing alone is not completeness.
        if part.subparts.is_empty() {
            issues.push(format!("part {id}: multipart_has_no_parts"));
        }
        match part.ctype.params.get("boundary") {
            Some(boundary) => {
                let start = format!("--{boundary}");
                if !part
                    .raw_bytes
                    .split(|byte| *byte == b'\n')
                    .any(|line| line.trim_ascii_end() == start.as_bytes())
                {
                    issues.push(format!("part {id}: missing_opening_multipart_boundary"));
                }
                let end = format!("--{boundary}--");
                if !part
                    .raw_bytes
                    .split(|byte| *byte == b'\n')
                    .any(|line| line.trim_ascii_end() == end.as_bytes())
                {
                    issues.push(format!("part {id}: missing_closing_multipart_boundary"));
                }
            }
            None => issues.push(format!("part {id}: missing_multipart_boundary")),
        }
    }
    if depth >= 100 {
        issues.push(format!("part {id}: nesting_limit_reached"));
        return;
    }
    for (index, child) in part.subparts.iter().enumerate() {
        collect_parts(
            child,
            &format!("{id}.{}", index + 1),
            Some(id),
            depth + 1,
            parts,
            issues,
        );
    }
    if part.ctype.mimetype.eq_ignore_ascii_case("message/rfc822") && part.subparts.is_empty() {
        if let Some(bytes) = decoded {
            match mailparse::parse_mail(&bytes) {
                Ok(child) => collect_parts(&child, &format!("{id}.1"), Some(id), depth + 1, parts, issues),
                Err(error) => issues.push(format!("part {id}: embedded_message_parse_failed: {error}")),
            }
        }
    }
}

fn synthetic_mime(body: &str) -> Vec<u8> {
    format!(
        "From: Builder <builder@example.invalid>\r\nTo: owner@example.invalid\r\n\
         Subject: Room Renovation\r\nMessage-ID: <room@example.invalid>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=synthetic-room\r\n\r\n\
         --synthetic-room\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{body}\r\n\
         --synthetic-room\r\nContent-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"plan.bin\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nAAEC/w==\r\n\
         --synthetic-room\r\nContent-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"plan.bin\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nAwQF/g==\r\n--synthetic-room--\r\n"
    ).into_bytes()
}
