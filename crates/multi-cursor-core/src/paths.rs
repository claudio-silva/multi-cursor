use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use std::path::Path;

#[cfg(target_os = "macos")]
use crate::config::AppConfig;

pub const AUTH_KEYS: &[&str] = &[
    "cursorAuth/accessToken",
    "cursorAuth/refreshToken",
    "cursorAuth/cachedEmail",
    "cursorAuth/cachedSignUpType",
    "cursorAuth/stripeMembershipType",
    "cursorAuth/stripeSubscriptionStatus",
    "cursorAuth/cachedScopedProfile",
];

/// Inactive Application Support folder: `Cursor.multi-cursor.<envId>`
#[cfg(target_os = "macos")]
pub const DATA_INACTIVE_PREFIX: &str = "Cursor.multi-cursor.";
/// Inactive home folder: `.cursor.multi-cursor.<envId>`
#[cfg(target_os = "macos")]
pub const DOT_INACTIVE_PREFIX: &str = ".cursor.multi-cursor.";
#[cfg(target_os = "macos")]
const SWAP_TOKEN: &str = "__swap__";

pub fn default_cursor_app_path() -> String {
    #[cfg(target_os = "macos")]
    {
        "/Applications/Cursor.app".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        String::new()
    }
}

pub fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

pub fn root_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".multi-cursor"))
}

pub fn config_path() -> Result<PathBuf, String> {
    Ok(root_dir()?.join("config.json"))
}

/// Active `~/.cursor` (extensions / CLI state).
pub fn active_dot_cursor_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".cursor"))
}

pub fn account_snapshot_path(env_id: &str, account_id: &str) -> Result<PathBuf, String> {
    Ok(root_dir()?
        .join("accounts")
        .join(env_id)
        .join(format!("{account_id}.json")))
}

pub fn ensure_layout() -> Result<(), String> {
    let root = root_dir()?;
    fs::create_dir_all(root.join("accounts")).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn now_iso() -> String {
    chrono::Local::now().to_rfc3339()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(target_os = "macos")]
pub fn application_support_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join("Library").join("Application Support"))
}

#[cfg(target_os = "macos")]
pub fn active_data_dir() -> Result<PathBuf, String> {
    Ok(application_support_dir()?.join("Cursor"))
}

#[cfg(target_os = "macos")]
pub fn inactive_data_dir(env_id: &str) -> Result<PathBuf, String> {
    Ok(application_support_dir()?.join(format!("{DATA_INACTIVE_PREFIX}{env_id}")))
}

#[cfg(target_os = "macos")]
pub fn inactive_dot_cursor_dir(env_id: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?.join(format!("{DOT_INACTIVE_PREFIX}{env_id}")))
}

#[cfg(target_os = "macos")]
pub fn env_data_dir(env_id: &str, active_env_id: Option<&str>) -> Result<PathBuf, String> {
    if active_env_id == Some(env_id) {
        active_data_dir()
    } else {
        inactive_data_dir(env_id)
    }
}

#[cfg(target_os = "macos")]
pub fn env_dot_cursor_dir(env_id: &str, active_env_id: Option<&str>) -> Result<PathBuf, String> {
    if active_env_id == Some(env_id) {
        active_dot_cursor_dir()
    } else {
        inactive_dot_cursor_dir(env_id)
    }
}

#[cfg(target_os = "macos")]
pub fn env_state_db(env_id: &str, active_env_id: Option<&str>) -> Result<PathBuf, String> {
    Ok(env_data_dir(env_id, active_env_id)?
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

#[cfg(target_os = "macos")]
pub fn prepare_inactive_environment(env_id: &str) -> Result<(), String> {
    let data = inactive_data_dir(env_id)?;
    fs::create_dir_all(data.join("User").join("globalStorage")).map_err(|e| e.to_string())?;
    fs::create_dir_all(inactive_dot_cursor_dir(env_id)?).map_err(|e| e.to_string())?;
    fs::create_dir_all(root_dir()?.join("accounts").join(env_id)).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn storage_v2_marker() -> Result<PathBuf, String> {
    Ok(root_dir()?.join(".storage-v2"))
}

#[cfg(target_os = "macos")]
pub fn migrate_legacy_storage(cfg: &AppConfig) -> Result<(), String> {
    let marker = storage_v2_marker()?;
    if marker.exists() {
        return Ok(());
    }

    if let Some(active_id) = cfg.active.env_id.as_deref() {
        let legacy = root_dir()?.join("environments").join(active_id);
        if legacy.exists() {
            let live = active_data_dir()?;
            let live_dot = active_dot_cursor_dir()?;
            let stock_id = new_id();
            let stock = format!("_pre-v2-{}", &stock_id[..8]);
            if live.exists() {
                let park = inactive_data_dir(&stock)?;
                if !park.exists() {
                    rename_path(&live, &park)?;
                }
            }
            if live_dot.exists() {
                let park_dot = inactive_dot_cursor_dir(&stock)?;
                if !park_dot.exists() {
                    rename_path(&live_dot, &park_dot)?;
                }
            }
            rename_path(&legacy, &live)?;
            let nested_ext = live.join("extensions");
            if nested_ext.exists() {
                fs::create_dir_all(&live_dot).map_err(|e| e.to_string())?;
                let dest_ext = live_dot.join("extensions");
                if dest_ext.exists() {
                    let _ = move_to_trash(&dest_ext);
                }
                rename_path(&nested_ext, &dest_ext)?;
            } else {
                fs::create_dir_all(&live_dot).map_err(|e| e.to_string())?;
            }
        }
    }

    fs::write(&marker, "2\n").map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn ensure_environment_pool(cfg: &AppConfig) -> Result<(), String> {
    let active = cfg.active.env_id.as_deref();
    ensure_active_dirs()?;
    for env in &cfg.environments {
        fs::create_dir_all(root_dir()?.join("accounts").join(&env.id))
            .map_err(|e| e.to_string())?;
        if Some(env.id.as_str()) == active {
            continue;
        }
        let data = inactive_data_dir(&env.id)?;
        let dot = inactive_dot_cursor_dir(&env.id)?;
        if data.exists() && dot.exists() {
            continue;
        }
        let legacy = root_dir()?.join("environments").join(&env.id);
        if legacy.exists() {
            if !data.exists() {
                copy_dir_recursive(&legacy, &data)?;
            }
            if !dot.exists() {
                let legacy_ext = legacy.join("extensions");
                fs::create_dir_all(&dot).map_err(|e| e.to_string())?;
                if legacy_ext.exists() {
                    copy_dir_recursive(&legacy_ext, &dot.join("extensions"))?;
                }
            }
        } else {
            prepare_inactive_environment(&env.id)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn ensure_active_dirs() -> Result<(), String> {
    let data = active_data_dir()?;
    fs::create_dir_all(data.join("User").join("globalStorage")).map_err(|e| e.to_string())?;
    fs::create_dir_all(active_dot_cursor_dir()?).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() {
        return Err(format!("Source does not exist: {}", src.display()));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if dst.exists() {
        fs::remove_dir_all(dst).map_err(|e| format!("Clear destination {}: {e}", dst.display()))?;
    }

    let status = std::process::Command::new("ditto")
        .arg(src)
        .arg(dst)
        .status()
        .map_err(|e| format!("ditto failed to start: {e}"))?;
    if status.success() {
        return Ok(());
    }

    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn rename_path(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!(
            "Cannot rename {} → {} (destination exists)",
            from.display(),
            to.display()
        ));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(from, to).map_err(|e| {
        format!(
            "Failed to rename {} → {}: {e}",
            from.display(),
            to.display()
        )
    })
}

#[cfg(target_os = "macos")]
pub fn activate_environment_dirs(
    from_env_id: Option<&str>,
    to_env_id: &str,
) -> Result<(), String> {
    if from_env_id == Some(to_env_id) {
        return Ok(());
    }

    let to_data = inactive_data_dir(to_env_id)?;
    let to_dot = inactive_dot_cursor_dir(to_env_id)?;
    if !to_data.exists() {
        return Err(format!(
            "Environment data folder missing: {}",
            to_data.display()
        ));
    }
    if !to_dot.exists() {
        return Err(format!(
            "Environment ~/.cursor folder missing: {}",
            to_dot.display()
        ));
    }

    let active_data = active_data_dir()?;
    let active_dot = active_dot_cursor_dir()?;

    let (park_data, park_dot) = if let Some(from_id) = from_env_id {
        let dest_data = inactive_data_dir(from_id)?;
        let dest_dot = inactive_dot_cursor_dir(from_id)?;
        if dest_data.exists() {
            return Err(format!(
                "Inactive data folder already exists for active env: {}",
                dest_data.display()
            ));
        }
        if dest_dot.exists() {
            return Err(format!(
                "Inactive ~/.cursor folder already exists for active env: {}",
                dest_dot.display()
            ));
        }
        (dest_data, dest_dot)
    } else {
        let swap_suffix = format!("{SWAP_TOKEN}-{}", new_id());
        (
            application_support_dir()?.join(format!("{DATA_INACTIVE_PREFIX}{swap_suffix}")),
            home_dir()?.join(format!("{DOT_INACTIVE_PREFIX}{swap_suffix}")),
        )
    };

    if active_data.exists() {
        rename_path(&active_data, &park_data)?;
    } else {
        fs::create_dir_all(&park_data).map_err(|e| e.to_string())?;
        fs::create_dir_all(park_data.join("User").join("globalStorage"))
            .map_err(|e| e.to_string())?;
    }
    if active_dot.exists() {
        rename_path(&active_dot, &park_dot)?;
    } else {
        fs::create_dir_all(&park_dot).map_err(|e| e.to_string())?;
    }

    rename_path(&to_data, &active_data)?;
    rename_path(&to_dot, &active_dot)?;

    Ok(())
}

#[cfg(target_os = "macos")]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let trash_cli = std::process::Command::new("trash").arg(path).output();
    if let Ok(out) = trash_cli {
        if out.status.success() {
            return Ok(());
        }
    }
    let posix = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string();
    let script = format!(
        "tell application \"Finder\" to delete (POSIX file \"{}\" as alias)",
        posix.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Failed to move to Trash: {}", path.display()))
    }
}
