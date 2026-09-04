use std::collections::HashMap;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::paths::account_snapshot_path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthSnapshot {
    pub keys: HashMap<String, String>,
}

pub fn save_snapshot(env_id: &str, account_id: &str, snap: &AuthSnapshot) -> Result<(), String> {
    let path = account_snapshot_path(env_id, account_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(snap).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

pub fn load_snapshot(env_id: &str, account_id: &str) -> Result<AuthSnapshot, String> {
    let path = account_snapshot_path(env_id, account_id)?;
    if !path.exists() {
        return Ok(AuthSnapshot::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("Invalid account snapshot: {e}"))
}

pub fn delete_snapshot(env_id: &str, account_id: &str) -> Result<(), String> {
    let path = account_snapshot_path(env_id, account_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn email_from_snapshot(snap: &AuthSnapshot) -> Option<String> {
    snap.keys
        .get("cursorAuth/cachedEmail")
        .cloned()
        .filter(|s| !s.is_empty())
}

/// Display name from `cursorAuth/cachedScopedProfile` JSON (`displayName` or `name`).
pub fn profile_name_from_snapshot(snap: &AuthSnapshot) -> Option<String> {
    let raw = snap.keys.get("cursorAuth/cachedScopedProfile")?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("displayName")
        .or_else(|| value.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Prefer profile display name; fall back to email.
pub fn display_name_from_snapshot(snap: &AuthSnapshot) -> Option<String> {
    profile_name_from_snapshot(snap).or_else(|| email_from_snapshot(snap))
}

pub fn has_login_tokens(snap: &AuthSnapshot) -> bool {
    snap.keys
        .get("cursorAuth/accessToken")
        .map(|t| !t.is_empty())
        .unwrap_or(false)
}

/// Build a snapshot from raw tokens and optional identity fields.
pub fn snapshot_from_tokens(
    access: &str,
    refresh: &str,
    email: Option<&str>,
    display_name: Option<&str>,
) -> AuthSnapshot {
    let mut keys = HashMap::new();
    keys.insert("cursorAuth/accessToken".to_string(), access.to_string());
    keys.insert("cursorAuth/refreshToken".to_string(), refresh.to_string());
    if let Some(email) = email.filter(|s| !s.is_empty()) {
        keys.insert("cursorAuth/cachedEmail".to_string(), email.to_string());
    }
    if let Some(name) = display_name.filter(|s| !s.is_empty()) {
        let profile = serde_json::json!({ "displayName": name });
        keys.insert(
            "cursorAuth/cachedScopedProfile".to_string(),
            profile.to_string(),
        );
    }
    AuthSnapshot { keys }
}
