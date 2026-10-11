//! Deterministic source export and dossiers referencing existing records.
use std::path::Path;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::db::Database;
use crate::models::error::{AppError, Result};

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

pub fn source(_db: &Database, _account_id: &str, _email_id: &str) -> Result<Value> {
    Err(AppError::InvalidInput("source prototype not implemented".into()))
}

pub fn project(_db: &Database, _account_id: &str, _manifest: &ProjectManifest) -> Result<Value> {
    Err(AppError::InvalidInput("project prototype not implemented".into()))
}

pub fn seed(_data_dir: &Path) -> Result<Value> {
    Err(AppError::InvalidInput("synthetic seed not implemented".into()))
}
