//! Sync Multi Cursor account tokens into the stores Cursor Agent CLI reads.
//!
//! - macOS: Keychain (`cursor-access-token` / `cursor-refresh-token`) + `cli-config.json`
//! - Linux: `~/.config/cursor/auth.json` + `cli-config.json`

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::paths::{active_dot_cursor_dir, home_dir};
use crate::snapshot::{
    display_name_from_snapshot, email_from_snapshot, has_login_tokens, snapshot_from_tokens,
    AuthSnapshot,
};

fn cli_config_path() -> Result<PathBuf, String> {
    let live = active_dot_cursor_dir()?.join("cli-config.json");
    if live.exists() {
        return Ok(live);
    }
    Ok(home_dir()?.join(".cursor").join("cli-config.json"))
}

fn update_cli_config_auth_info(auth_info: Option<Value>) -> Result<(), String> {
    let path = cli_config_path()?;
    if !path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(&path).map_err(|e| format!("Read cli-config.json: {e}"))?;
    let mut value: Value =
        serde_json::from_str(&raw).map_err(|e| format!("Parse cli-config.json: {e}"))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| "cli-config.json root must be an object".to_string())?;

    match auth_info {
        Some(info) => {
            obj.insert("authInfo".to_string(), info);
        }
        None => {
            obj.remove("authInfo");
        }
    }

    let pretty = serde_json::to_string_pretty(&value)
        .map_err(|e| format!("Serialize cli-config.json: {e}"))?;
    fs::write(&path, format!("{pretty}\n")).map_err(|e| format!("Write cli-config.json: {e}"))?;
    Ok(())
}

fn auth_info_from_snapshot(snap: &AuthSnapshot) -> Value {
    let mut map = Map::new();
    if let Some(email) = email_from_snapshot(snap) {
        map.insert("email".to_string(), json!(email));
    }
    if let Some(name) = display_name_from_snapshot(snap) {
        map.insert("displayName".to_string(), json!(name));
    }
    Value::Object(map)
}

fn read_cli_config_identity() -> (Option<String>, Option<String>) {
    let Ok(path) = cli_config_path() else {
        return (None, None);
    };
    if !path.exists() {
        return (None, None);
    }
    let Ok(raw) = fs::read_to_string(&path) else {
        return (None, None);
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return (None, None);
    };
    let info = value.get("authInfo");
    let email = info
        .and_then(|v| v.get("email"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let display_name = info
        .and_then(|v| v.get("displayName"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    (email, display_name)
}

/// Push account tokens into the platform credential store + `cli-config.json`.
pub fn sync_cli_auth_from_snapshot(snap: &AuthSnapshot) -> Result<(), String> {
    if !has_login_tokens(snap) {
        return clear_cli_auth();
    }

    let access = snap
        .keys
        .get("cursorAuth/accessToken")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing access token in account snapshot".to_string())?;
    let refresh = snap
        .keys
        .get("cursorAuth/refreshToken")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing refresh token in account snapshot".to_string())?;

    write_platform_tokens(access, refresh)?;
    update_cli_config_auth_info(Some(auth_info_from_snapshot(snap)))?;
    Ok(())
}

/// Clear Agent CLI credentials for the current platform.
pub fn clear_cli_auth() -> Result<(), String> {
    clear_platform_tokens()?;
    update_cli_config_auth_info(None)?;
    Ok(())
}

/// Read the currently active Cursor Agent login into an AuthSnapshot.
pub fn read_live_agent_snapshot() -> Result<AuthSnapshot, String> {
    let (access, refresh) = read_platform_tokens()?;
    let (email, display_name) = read_cli_config_identity();
    Ok(snapshot_from_tokens(
        &access,
        &refresh,
        email.as_deref(),
        display_name.as_deref(),
    ))
}

#[cfg(target_os = "macos")]
mod platform {
    use std::process::Command;

    const KEYCHAIN_ACCOUNT: &str = "cursor-user";
    const ACCESS_TOKEN_SERVICE: &str = "cursor-access-token";
    const REFRESH_TOKEN_SERVICE: &str = "cursor-refresh-token";

    fn security_output(args: &[&str]) -> Result<std::process::Output, String> {
        Command::new("security")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run security {}: {e}", args.first().unwrap_or(&"")))
    }

    fn delete_keychain_password(service: &str) -> Result<(), String> {
        let output = security_output(&[
            "delete-generic-password",
            "-s",
            service,
            "-a",
            KEYCHAIN_ACCOUNT,
        ])?;
        if output.status.success() || output.status.code() == Some(44) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not be found")
            || stderr.contains("The specified item could not be found")
        {
            return Ok(());
        }
        Err(format!(
            "Failed to delete Keychain item {service}: {}",
            stderr.trim()
        ))
    }

    fn set_keychain_password(service: &str, password: &str) -> Result<(), String> {
        delete_keychain_password(service)?;
        let output = security_output(&[
            "add-generic-password",
            "-s",
            service,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
            password,
            "-U",
        ])?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "Failed to write Keychain item {service}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }

    fn get_keychain_password(service: &str) -> Result<Option<String>, String> {
        let output = security_output(&[
            "find-generic-password",
            "-s",
            service,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
        ])?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    pub fn write_platform_tokens(access: &str, refresh: &str) -> Result<(), String> {
        set_keychain_password(ACCESS_TOKEN_SERVICE, access)?;
        set_keychain_password(REFRESH_TOKEN_SERVICE, refresh)?;
        Ok(())
    }

    pub fn clear_platform_tokens() -> Result<(), String> {
        delete_keychain_password(ACCESS_TOKEN_SERVICE)?;
        delete_keychain_password(REFRESH_TOKEN_SERVICE)?;
        Ok(())
    }

    pub fn read_platform_tokens() -> Result<(String, String), String> {
        let access = get_keychain_password(ACCESS_TOKEN_SERVICE)?
            .ok_or_else(|| "No cursor-access-token in Keychain".to_string())?;
        let refresh = get_keychain_password(REFRESH_TOKEN_SERVICE)?
            .ok_or_else(|| "No cursor-refresh-token in Keychain".to_string())?;
        Ok((access, refresh))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::PathBuf;

    use serde_json::{json, Value};

    use crate::paths::home_dir;

    pub fn auth_json_path() -> Result<PathBuf, String> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().unwrap_or_default().join(".config"));
        if base.as_os_str().is_empty() {
            return Err("Cannot resolve XDG config home".to_string());
        }
        Ok(base.join("cursor").join("auth.json"))
    }

    fn read_auth_json() -> Result<Value, String> {
        let path = auth_json_path()?;
        if !path.exists() {
            return Ok(json!({}));
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("Read auth.json: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("Parse auth.json: {e}"))
    }

    fn write_auth_json(value: &Value) -> Result<(), String> {
        let path = auth_json_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Create auth.json dir: {e}"))?;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
        let pretty = serde_json::to_string_pretty(value)
            .map_err(|e| format!("Serialize auth.json: {e}"))?;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        use std::io::Write;
        let mut file = opts
            .open(&path)
            .map_err(|e| format!("Open auth.json for write: {e}"))?;
        file.write_all(format!("{pretty}\n").as_bytes())
            .map_err(|e| format!("Write auth.json: {e}"))?;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        Ok(())
    }

    pub fn write_platform_tokens(access: &str, refresh: &str) -> Result<(), String> {
        let mut value = read_auth_json()?;
        let obj = value
            .as_object_mut()
            .ok_or_else(|| "auth.json root must be an object".to_string())?;
        obj.insert("accessToken".to_string(), json!(access));
        obj.insert("refreshToken".to_string(), json!(refresh));
        write_auth_json(&value)
    }

    pub fn clear_platform_tokens() -> Result<(), String> {
        let path = auth_json_path()?;
        if !path.exists() {
            return Ok(());
        }
        let mut value = read_auth_json()?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("accessToken");
            obj.remove("refreshToken");
            if obj.is_empty()
                || (obj.get("apiKey").is_none() && obj.get("bedrockCredentials").is_none())
            {
                // Keep optional fields if present; otherwise remove file.
                if obj.get("apiKey").is_none() && obj.get("bedrockCredentials").is_none() {
                    fs::remove_file(&path).map_err(|e| format!("Remove auth.json: {e}"))?;
                    return Ok(());
                }
            }
        }
        write_auth_json(&value)
    }

    pub fn read_platform_tokens() -> Result<(String, String), String> {
        let value = read_auth_json()?;
        let access = value
            .get("accessToken")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "No accessToken in ~/.config/cursor/auth.json".to_string())?
            .to_string();
        let refresh = value
            .get("refreshToken")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "No refreshToken in ~/.config/cursor/auth.json".to_string())?
            .to_string();
        Ok((access, refresh))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn write_platform_tokens(_access: &str, _refresh: &str) -> Result<(), String> {
        Err("Cursor Agent auth sync is only supported on macOS and Linux".to_string())
    }

    pub fn clear_platform_tokens() -> Result<(), String> {
        Err("Cursor Agent auth sync is only supported on macOS and Linux".to_string())
    }

    pub fn read_platform_tokens() -> Result<(String, String), String> {
        Err("Cursor Agent auth sync is only supported on macOS and Linux".to_string())
    }
}

use platform::{clear_platform_tokens, read_platform_tokens, write_platform_tokens};
