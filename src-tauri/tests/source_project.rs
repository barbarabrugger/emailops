//! Synthetic-only native contracts. No provider, model, credential or network is used.
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use emailops_lib::db::Database;
use emailops_lib::models::error::AppError;
use emailops_lib::services::source_project::{project, seed, source, ProjectManifest};
use rusqlite::params;
use serde_json::json;
#[cfg(feature = "cli")]
use serde_json::Value;
use sha2::{Digest, Sha256};

fn raw_mime() -> Vec<u8> {
    concat!(
        "From: Builder <builder@example.invalid>\r\n",
        "To: owner@example.invalid\r\nSubject: Room Renovation\r\n",
        "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=fixture\r\n\r\n",
        "--fixture\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",
        "Full original text.\r\n",
        "--fixture\r\nContent-Type: application/octet-stream\r\n",
        "Content-Disposition: attachment; filename=\"plan.bin\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\nAAEC/w==\r\n",
        "--fixture\r\nContent-Type: application/octet-stream\r\n",
        "Content-Disposition: attachment; filename=\"plan.bin\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\nAwQF/g==\r\n--fixture--\r\n"
    )
    .as_bytes()
    .to_vec()
}

fn fixture() -> (tempfile::TempDir, Database, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::new(dir.path().to_path_buf()).unwrap();
    let body = format!("{}TAIL-UNTRUNCATED", "Renovation ä 🛠\r\n".repeat(3000));
    {
        let conn = db.connection();
        for (account, mail) in [("synthetic", "mail-full"), ("other", "mail-other")] {
            conn.execute(
                "INSERT INTO accounts(id,provider,email,name,created_at) VALUES(?1,'imap',?2,'Synthetic',0)",
                params![account, format!("{account}@example.invalid")],
            )
            .unwrap();
            conn.execute("INSERT INTO emails(id,account_id,thread_id,subject,sender,sender_email,recipients_json,snippet,timestamp,created_at)
                VALUES(?1,?2,'thread-room','Room Renovation','Builder','builder@example.invalid','[]','short',0,0)",
                params![mail, account]).unwrap();
            conn.execute(
                "INSERT INTO email_bodies(email_id,body) VALUES(?1,?2)",
                params![mail, body],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pending_tasks(id,account_id,title,source_email_id,due_at,created_at,updated_at)
                VALUES(?1,?2,'Confirm materials',?3,1800000000,0,0)",
                params![format!("task-{account}"), account, mail],
            )
            .unwrap();
            conn.execute("INSERT INTO calendar_events(id,account_id,provider_event_id,title,start_time,end_time,created_at,updated_at)
                VALUES(?1,?2,?1,'Site visit',1800000000,1800003600,0,0)",
                params![format!("event-{account}"), account]).unwrap();
        }
        conn.execute(
            "INSERT INTO original_email_sources(email_id,raw_mime,provenance,captured_at)
            VALUES('mail-full',?1,'synthetic fixture',123)",
            params![raw_mime()],
        )
        .unwrap();
    }
    (dir, db, body)
}

fn manifest() -> ProjectManifest {
    ProjectManifest {
        schema_version: 1,
        id: "room-renovation".into(),
        title: "Room Renovation".into(),
        account_id: "synthetic".into(),
        email_ids: vec!["mail-full".into()],
        task_ids: vec!["task-synthetic".into()],
        event_ids: vec!["event-synthetic".into()],
    }
}

#[test]
fn cached_body_is_exact_and_never_claims_to_be_the_original() {
    let (_dir, db, body) = fixture();
    let value = source(&db, "synthetic", "mail-full").unwrap();
    assert_eq!(value["cached_body"]["text"], body);
    assert_eq!(
        value["cached_body"]["sha256"],
        hex::encode(Sha256::digest(body.as_bytes()))
    );
    assert_eq!(value["cached_body"]["original_source"], false);
}

#[test]
fn source_preserves_raw_bytes_and_duplicate_filenames_as_distinct_parts() {
    let (_dir, db, _) = fixture();
    let value = source(&db, "synthetic", "mail-full").unwrap();
    assert_eq!(
        STANDARD.decode(value["raw_mime"]["base64"].as_str().unwrap()).unwrap(),
        raw_mime()
    );
    assert_eq!(value["raw_mime"]["sha256"], hex::encode(Sha256::digest(raw_mime())));
    assert_eq!(value["raw_mime"]["provenance"], "synthetic fixture");
    assert_eq!(value["completeness"]["original_source_complete"], true);
    let parts = value["mime_parts"].as_array().unwrap();
    assert_eq!(parts.len(), 4); // root + body + both attachments
    let attachments: Vec<_> = parts.iter().filter(|p| p["filename"] == "plan.bin").collect();
    assert_eq!(attachments.len(), 2);
    assert_ne!(attachments[0]["id"], attachments[1]["id"]);
    assert_eq!(
        STANDARD
            .decode(attachments[0]["decoded_base64"].as_str().unwrap())
            .unwrap(),
        [0, 1, 2, 255]
    );
    assert_eq!(
        STANDARD
            .decode(attachments[1]["decoded_base64"].as_str().unwrap())
            .unwrap(),
        [3, 4, 5, 254]
    );
    assert_ne!(attachments[0]["decoded_sha256"], attachments[1]["decoded_sha256"]);
}

#[test]
fn missing_original_and_missing_cached_body_are_explicit() {
    let (_dir, db, _) = fixture();
    db.connection()
        .execute("DELETE FROM email_bodies WHERE email_id='mail-other'", [])
        .unwrap();
    let value = source(&db, "other", "mail-other").unwrap();
    assert_eq!(value["raw_mime"]["available"], false);
    assert_eq!(value["cached_body"]["available"], false);
    assert_eq!(value["completeness"]["missing_raw"], true);
    assert_eq!(value["completeness"]["original_source_complete"], false);
    assert_eq!(value["mime_parts"], json!([]));
}

#[test]
fn source_cannot_read_another_accounts_message() {
    let (_dir, db, _) = fixture();
    assert!(matches!(
        source(&db, "synthetic", "mail-other"),
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn dossier_links_existing_tasks_and_events_without_creating_new_rows() {
    let (_dir, db, _) = fixture();
    let first = project(&db, "synthetic", &manifest()).unwrap();
    assert_eq!(first["email_sources"][0]["email_id"], "mail-full");
    assert_eq!(first["tasks"][0]["id"], "task-synthetic");
    assert_eq!(first["tasks"][0]["sourceEmailId"], "mail-full");
    assert_eq!(first["tasks"][0]["dueAt"], 1800000000i64);
    assert_eq!(first["events"][0]["id"], "event-synthetic");
    assert_eq!(first["events"][0]["startTime"], 1800000000i64);
    assert_eq!(first["missing_references"], json!([]));
    assert_eq!(first, project(&db, "synthetic", &manifest()).unwrap());
    assert_eq!(first["content_sha256"].as_str().unwrap().len(), 64);
    let count: i64 = db
        .reader()
        .query_row("SELECT COUNT(*) FROM pending_tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    db.connection()
        .execute("UPDATE pending_tasks SET status='done' WHERE id='task-synthetic'", [])
        .unwrap();
    assert_ne!(
        first["content_sha256"],
        project(&db, "synthetic", &manifest()).unwrap()["content_sha256"]
    );
}

#[test]
fn missing_and_cross_account_references_are_visible_without_leaking_records() {
    let (_dir, db, _) = fixture();
    let mut m = manifest();
    m.email_ids.push("mail-other".into());
    m.task_ids.push("task-other".into());
    m.event_ids.push("event-other".into());
    m.email_ids.push("absent".into());
    let value = project(&db, "synthetic", &m).unwrap();
    assert_eq!(value["complete"], false);
    assert_eq!(value["missing_references"].as_array().unwrap().len(), 4);
    assert_eq!(value["email_sources"].as_array().unwrap().len(), 1);
    assert_eq!(value["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(value["events"].as_array().unwrap().len(), 1);
}

#[test]
fn account_mismatch_and_unknown_manifest_version_are_rejected() {
    let (_dir, db, _) = fixture();
    assert!(matches!(
        project(&db, "other", &manifest()),
        Err(AppError::InvalidInput(_))
    ));
    let mut m = manifest();
    m.schema_version = 2;
    assert!(matches!(project(&db, "synthetic", &m), Err(AppError::InvalidInput(_))));
}

#[test]
fn synthetic_seed_requires_empty_destination_and_uses_real_schema() {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("new");
    let result = seed(&dir).unwrap();
    assert_eq!(result["account_id"], "synthetic");
    assert!(dir.join("project-manifest.json").is_file());
    let db = Database::new(dir.clone()).unwrap();
    let m: ProjectManifest =
        serde_json::from_slice(&std::fs::read(dir.join("project-manifest.json")).unwrap()).unwrap();
    assert!(project(&db, "synthetic", &m).unwrap()["complete"].as_bool().unwrap());
    assert!(seed(&dir).is_err());
    let occupied = parent.path().join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("keep.txt"), b"keep").unwrap();
    assert!(seed(&occupied).is_err());
    assert!(!occupied.join("emailops.db").exists());
    assert_eq!(std::fs::read(occupied.join("keep.txt")).unwrap(), b"keep");
}

#[cfg(feature = "cli")]
fn cli(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_emailops-cli"))
        .args(args)
        .env("DBUS_SESSION_BUS_ADDRESS", "disabled-for-synthetic-contract")
        .output()
        .unwrap()
}

#[cfg(feature = "cli")]
#[test]
fn cli_source_is_readonly_account_scoped_and_needs_no_keychain() {
    let (dir, db, body) = fixture();
    drop(db);
    let path = dir.path().to_str().unwrap();
    let before = std::fs::read(dir.path().join("emailops.db")).unwrap();
    let result = cli(&[
        "--json",
        "--data-dir",
        path,
        "--account",
        "synthetic",
        "source",
        "mail-full",
    ]);
    let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(result.status.success(), "{envelope}");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["cached_body"]["text"], body);
    assert_eq!(before, std::fs::read(dir.path().join("emailops.db")).unwrap());
    let denied = cli(&[
        "--json",
        "--data-dir",
        path,
        "--account",
        "synthetic",
        "source",
        "mail-other",
    ]);
    assert_eq!(denied.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "not_found");
}

#[cfg(feature = "cli")]
#[test]
fn cli_seed_requires_explicit_data_dir_and_dossier_roundtrips() {
    let missing = cli(&["--json", "prototype-seed"]);
    let error: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(error["error"]["code"], "invalid_input");
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("new");
    let path = dir.to_str().unwrap();
    let seeded = cli(&["--json", "--data-dir", path, "prototype-seed"]);
    assert!(seeded.status.success(), "{}", String::from_utf8_lossy(&seeded.stdout));
    let manifest = dir.join("project-manifest.json");
    let args = [
        "--json",
        "--data-dir",
        path,
        "--account",
        "synthetic",
        "project",
        "--manifest",
        manifest.to_str().unwrap(),
    ];
    let first = cli(&args);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stdout));
    assert_eq!(first.stdout, cli(&args).stdout);
    let result: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(result["data"]["manifest"]["title"], "Room Renovation");
    assert_eq!(result["data"]["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn invalid_transfer_encoding_keeps_original_bytes_and_reports_incomplete() {
    let (_dir, db, _) = fixture();
    let raw = b"Subject: Broken fixture\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n%%%invalid%%%";
    db.connection()
        .execute(
            "UPDATE original_email_sources SET raw_mime=?1 WHERE email_id='mail-full'",
            params![raw.as_slice()],
        )
        .unwrap();
    let value = source(&db, "synthetic", "mail-full").unwrap();
    assert_eq!(
        STANDARD.decode(value["raw_mime"]["base64"].as_str().unwrap()).unwrap(),
        raw
    );
    assert_eq!(value["completeness"]["original_source_complete"], false);
    assert!(!value["completeness"]["issues"].as_array().unwrap().is_empty());
}

#[test]
fn pre_capture_schema_is_readable_and_explicitly_missing_original_source() {
    let (_dir, db, body) = fixture();
    db.connection()
        .execute("DROP TABLE original_email_sources", [])
        .unwrap();
    let value = source(&db, "synthetic", "mail-full").unwrap();
    assert_eq!(value["cached_body"]["text"], body);
    assert_eq!(value["raw_mime"]["available"], false);
    assert_eq!(value["completeness"]["raw_capture_supported"], false);
    assert_eq!(value["completeness"]["original_source_complete"], false);
}

#[test]
fn nested_message_parts_and_inline_content_have_positional_ids() {
    let (_dir, db, _) = fixture();
    let raw = concat!(
        "Content-Type: multipart/mixed; boundary=outer\r\n\r\n",
        "--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n",
        "--inner\r\nContent-Type: text/plain\r\n\r\nplain\r\n",
        "--inner\r\nContent-Type: text/html\r\n\r\n<b>html</b>\r\n--inner--\r\n",
        "--outer\r\nContent-Type: message/rfc822\r\n\r\n",
        "From: nested@example.invalid\r\nContent-Type: text/plain\r\n\r\nembedded body\r\n",
        "--outer\r\nContent-Type: image/png\r\nContent-ID: <inline-one>\r\n",
        "Content-Disposition: inline; filename=\"inline.png\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\nAAEC\r\n--outer--\r\n"
    );
    db.connection()
        .execute(
            "UPDATE original_email_sources SET raw_mime=?1 WHERE email_id='mail-full'",
            params![raw.as_bytes()],
        )
        .unwrap();
    let value = source(&db, "synthetic", "mail-full").unwrap();
    let parts = value["mime_parts"].as_array().unwrap();
    let ids: Vec<&str> = parts.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["1", "1.1", "1.1.1", "1.1.2", "1.2", "1.2.1", "1.3"]);
    assert_eq!(parts[5]["decoded_text"], "embedded body");
    assert_eq!(parts[6]["content_id"], "<inline-one>");
    assert_eq!(parts[6]["disposition"], "inline");
}

#[test]
fn truncated_multipart_is_flagged_incomplete() {
    let (_dir, db, _) = fixture();
    let raw = b"Content-Type: multipart/mixed; boundary=unfinished\r\n\r\n--unfinished\r\nContent-Type: text/plain\r\n\r\nbody without closing boundary";
    db.connection()
        .execute(
            "UPDATE original_email_sources SET raw_mime=?1 WHERE email_id='mail-full'",
            params![raw.as_slice()],
        )
        .unwrap();
    let value = source(&db, "synthetic", "mail-full").unwrap();
    assert_eq!(value["completeness"]["original_source_complete"], false);
    assert!(!value["completeness"]["issues"].as_array().unwrap().is_empty());
}

#[test]
fn manifest_reference_order_and_duplicates_do_not_change_dossier_hash() {
    let (_dir, db, _) = fixture();
    let mut m = manifest();
    m.email_ids
        .extend(["missing-z".into(), "missing-a".into(), "mail-full".into()]);
    let first = project(&db, "synthetic", &m).unwrap();
    m.email_ids.reverse();
    assert_eq!(first, project(&db, "synthetic", &m).unwrap());
}

#[test]
fn seed_accepts_an_existing_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    assert!(seed(dir.path()).is_ok());
}

#[cfg(unix)]
#[test]
fn seed_rejects_a_symlink_destination_without_writing_through_it() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let link = parent.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(seed(&link).is_err());
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
}
