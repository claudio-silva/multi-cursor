use serde::Serialize;
use tauri::AppHandle;

use crate::auth::{
    clear_auth_keys, delete_snapshot, display_name_from_snapshot, email_from_snapshot,
    has_login_tokens, load_snapshot, profile_name_from_snapshot, read_auth_keys, save_snapshot,
    write_auth_keys, AuthSnapshot,
};
use crate::copy_progress::{
    cleanup_partial_copy, copy_cursor_tree_with_progress, dir_size_bytes, emit_progress,
    purge_vscdb_bak_files, trash_paths_with_progress,
};
use crate::cursor::{cursor_running, force_quit_cursor, launch_cursor, request_quit_cursor};
use crate::paths::{
    activate_environment_dirs, active_data_dir, active_dot_cursor_dir, application_support_dir,
    ensure_active_dirs, ensure_environment_pool, env_data_dir, env_dot_cursor_dir, env_state_db,
    home_dir, inactive_data_dir, inactive_dot_cursor_dir, load_config, migrate_legacy_storage,
    move_to_trash, new_id, now_iso, prepare_inactive_environment, root_dir, save_config,
    Account, AppConfig, Environment, DATA_INACTIVE_PREFIX, DOT_INACTIVE_PREFIX,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub config: AppConfig,
    pub cursor_running: bool,
    pub root_dir: String,
}

fn state_from(cfg: AppConfig) -> Result<AppState, String> {
    Ok(AppState {
        config: cfg,
        cursor_running: cursor_running(),
        root_dir: root_dir()?.display().to_string(),
    })
}

fn active_env_id(cfg: &AppConfig) -> Option<&str> {
    cfg.active.env_id.as_deref()
}

fn state_db(cfg: &AppConfig, env_id: &str) -> Result<std::path::PathBuf, String> {
    env_state_db(env_id, active_env_id(cfg))
}

/// First launch: register the live Cursor / ~/.cursor folders as "Default"
/// and capture the signed-in account email when tokens are present.
fn bootstrap_default_environment(cfg: &mut AppConfig) -> Result<(), String> {
    ensure_active_dirs()?;
    fs_create_accounts_root()?;

    if !cfg.environments.is_empty() {
        if cfg.active.env_id.is_none() {
            cfg.active.env_id = cfg.environments.first().map(|e| e.id.clone());
            save_config(cfg)?;
        }
        return Ok(());
    }

    let id = new_id();
    cfg.environments.push(Environment {
        id: id.clone(),
        name: "Default".to_string(),
        created_at: now_iso(),
    });
    cfg.active.env_id = Some(id.clone());
    capture_default_account(cfg, &id)?;
    save_config(cfg)?;
    // Fresh install — no legacy --user-data-dir tree to migrate.
    let marker = root_dir()?.join(".storage-v2");
    if !marker.exists() {
        std::fs::write(&marker, "2\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn adopt_live_login_if_needed(cfg: &mut AppConfig) -> Result<(), String> {
    let Some(env_id) = cfg.active.env_id.clone() else {
        return Ok(());
    };
    if cfg.accounts.iter().any(|a| a.env_id == env_id) {
        return Ok(());
    }
    capture_default_account(cfg, &env_id)?;
    save_config(cfg)?;
    Ok(())
}

fn capture_default_account(cfg: &mut AppConfig, env_id: &str) -> Result<(), String> {
    let db = env_state_db(env_id, Some(env_id))?;
    let snap = read_auth_keys(&db)?;
    if !has_login_tokens(&snap) {
        return Ok(());
    }
    let account_id = new_id();
    save_snapshot(env_id, &account_id, &snap)?;
    let email = email_from_snapshot(&snap);
    let name = display_name_from_snapshot(&snap)
        .unwrap_or_else(|| "Signed-in account".to_string());
    cfg.accounts.push(Account {
        id: account_id.clone(),
        env_id: env_id.to_string(),
        name,
        email,
        updated_at: now_iso(),
        pending_login: false,
    });
    cfg.active.account_id = Some(account_id);
    Ok(())
}

fn fs_create_accounts_root() -> Result<(), String> {
    std::fs::create_dir_all(root_dir()?.join("accounts")).map_err(|e| e.to_string())
}

fn account_needs_identity(account: &Account) -> bool {
    account.pending_login
        || account.email.as_ref().map(|e| e.is_empty()).unwrap_or(true)
        || account.name.starts_with("Signing in")
}

fn apply_captured_identity(account: &mut Account, email: &str, snap: &AuthSnapshot) {
    account.pending_login = false;
    account.email = Some(email.to_string());
    account.name = profile_name_from_snapshot(snap)
        .unwrap_or_else(|| email.to_string());
    account.updated_at = now_iso();
}

fn refresh_account_identity_from_snapshot(account: &mut Account) {
    if account.pending_login {
        return;
    }
    let Ok(snap) = load_snapshot(&account.env_id, &account.id) else {
        return;
    };
    if let Some(email) = email_from_snapshot(&snap) {
        account.email = Some(email.clone());
        account.name = profile_name_from_snapshot(&snap).unwrap_or(email);
    } else if let Some(name) = profile_name_from_snapshot(&snap) {
        account.name = name;
    }
}

fn try_capture_pending(cfg: &mut AppConfig) -> Result<Vec<String>, String> {
    // Capture pending logins. Also heal accounts stuck after an earlier capture
    // that saw tokens before Cursor wrote cachedEmail (pending cleared, name left).
    let candidates: Vec<(String, String, bool)> = cfg
        .accounts
        .iter()
        .filter(|a| account_needs_identity(a))
        .map(|a| (a.env_id.clone(), a.id.clone(), a.pending_login))
        .collect();

    let mut captured_emails = Vec::new();
    for (env_id, account_id, was_pending) in candidates {
        if was_pending {
            let db = state_db(cfg, &env_id)?;
            let snap = match read_auth_keys(&db) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if !has_login_tokens(&snap) {
                continue;
            }
            let Some(email) = email_from_snapshot(&snap) else {
                // Tokens can appear before cachedEmail — keep waiting.
                continue;
            };
            save_snapshot(&env_id, &account_id, &snap)?;
            if let Some(account) = cfg.accounts.iter_mut().find(|a| a.id == account_id) {
                apply_captured_identity(account, &email, &snap);
            }
            captured_emails.push(email);
            cfg.active.env_id = Some(env_id);
            cfg.active.account_id = Some(account_id);
            continue;
        }

        // Non-pending heal: only trust this account's own snapshot (not the live DB,
        // which may belong to a different account in the same environment).
        let Ok(snap) = load_snapshot(&env_id, &account_id) else {
            continue;
        };
        let Some(email) = email_from_snapshot(&snap) else {
            continue;
        };
        if !has_login_tokens(&snap) {
            continue;
        }
        if let Some(account) = cfg.accounts.iter_mut().find(|a| a.id == account_id) {
            apply_captured_identity(account, &email, &snap);
        }
    }
    Ok(captured_emails)
}

fn save_active_account_tokens(cfg: &AppConfig, env_id: &str) -> Result<(), String> {
    let Some(account_id) = cfg.active.account_id.as_deref() else {
        return Ok(());
    };
    let Some(account) = cfg.accounts.iter().find(|a| a.id == account_id) else {
        return Ok(());
    };
    if account.env_id != env_id || account.pending_login {
        return Ok(());
    }
    let db = state_db(cfg, env_id)?;
    let snap = read_auth_keys(&db)?;
    if has_login_tokens(&snap) {
        save_snapshot(env_id, account_id, &snap)?;
    }
    Ok(())
}

/// Make `env_id` own the live Application Support/Cursor and ~/.cursor folders.
fn switch_environment(cfg: &mut AppConfig, env_id: &str) -> Result<(), String> {
    if !cfg.environments.iter().any(|e| e.id == env_id) {
        return Err("Environment not found".to_string());
    }
    if cfg.active.env_id.as_deref() == Some(env_id) {
        return Ok(());
    }
    if cursor_running() {
        return Err(
            "Cursor is still running. Wait for it to quit before switching environments."
                .to_string(),
        );
    }

    if let Some(current) = cfg.active.env_id.clone() {
        save_active_account_tokens(cfg, &current)?;
    }

    activate_environment_dirs(cfg.active.env_id.as_deref(), env_id)?;
    cfg.active.env_id = Some(env_id.to_string());
    // Account may belong to the previous env; clear until apply_account sets it.
    if cfg
        .active
        .account_id
        .as_ref()
        .and_then(|aid| cfg.accounts.iter().find(|a| a.id == *aid))
        .map(|a| a.env_id.as_str())
        != Some(env_id)
    {
        cfg.active.account_id = None;
    }
    save_config(cfg)?;
    Ok(())
}

/// Switch auth in the environment DB. Caller must ensure Cursor is not running.
fn apply_account(cfg: &mut AppConfig, env_id: &str, account_id: &str) -> Result<(), String> {
    let account = cfg
        .accounts
        .iter()
        .find(|a| a.id == account_id && a.env_id == env_id)
        .ok_or_else(|| "Account not found".to_string())?
        .clone();

    switch_environment(cfg, env_id)?;
    save_active_account_tokens(cfg, env_id)?;

    let db = state_db(cfg, env_id)?;
    if account.pending_login {
        clear_auth_keys(&db)?;
    } else {
        let snap = load_snapshot(env_id, account_id)?;
        write_auth_keys(&db, &snap)?;
    }

    cfg.active.env_id = Some(env_id.to_string());
    cfg.active.account_id = Some(account_id.to_string());
    save_config(cfg)?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListStateResult {
    pub state: AppState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_email: Option<String>,
}

#[tauri::command]
pub fn list_state() -> Result<ListStateResult, String> {
    let mut cfg = load_config()?;
    bootstrap_default_environment(&mut cfg)?;
    migrate_legacy_storage(&cfg)?;
    ensure_environment_pool(&cfg)?;
    // Reclaim disk from older Multi Cursor builds that full-copied state.vscdb.
    let _ = purge_vscdb_bak_files(
        &active_data_dir()?
            .join("User")
            .join("globalStorage"),
    );
    let _ = cleanup_orphan_pool_dirs(&cfg);
    adopt_live_login_if_needed(&mut cfg)?;
    let captured = try_capture_pending(&mut cfg)?;
    for account in &mut cfg.accounts {
        refresh_account_identity_from_snapshot(account);
    }
    save_config(&cfg)?;
    Ok(ListStateResult {
        state: state_from(cfg)?,
        captured_email: captured.into_iter().next(),
    })
}

/// Trash pool folders that are not referenced by any configured environment.
fn cleanup_orphan_pool_dirs(cfg: &AppConfig) -> Result<u64, String> {
    let known: std::collections::HashSet<&str> =
        cfg.environments.iter().map(|e| e.id.as_str()).collect();
    let mut reclaimed = 0u64;

    let support = application_support_dir()?;
    if let Ok(entries) = std::fs::read_dir(&support) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_prefix(DATA_INACTIVE_PREFIX) {
                // Keep temporary swap parks and unknown ids out of known set → trash.
                if !known.contains(id) {
                    let size = dir_size_bytes(&entry.path());
                    move_to_trash(&entry.path())?;
                    reclaimed = reclaimed.saturating_add(size);
                }
            }
        }
    }

    let home = home_dir()?;
    if let Ok(entries) = std::fs::read_dir(&home) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_prefix(DOT_INACTIVE_PREFIX) {
                if !known.contains(id) {
                    let size = dir_size_bytes(&entry.path());
                    move_to_trash(&entry.path())?;
                    reclaimed = reclaimed.saturating_add(size);
                }
            }
        }
    }

    Ok(reclaimed)
}

/// Disk usage (bytes) for an environment's data + ~/.cursor pool folders.
/// Runs `du` off the main thread so switching environments does not freeze the UI.
#[tauri::command]
pub async fn environment_disk_usage(env_id: String) -> Result<u64, String> {
    let cfg = load_config()?;
    if !cfg.environments.iter().any(|e| e.id == env_id) {
        return Err("Environment not found".to_string());
    }
    let active = cfg.active.env_id.as_deref();
    let data = env_data_dir(&env_id, active)?;
    let dot = env_dot_cursor_dir(&env_id, active)?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(dir_size_bytes(&data).saturating_add(dir_size_bytes(&dot)))
    })
    .await
    .map_err(|e| format!("Disk usage task failed: {e}"))?
}

/// Create an empty inactive environment and return immediately (no copy).
#[tauri::command]
pub fn create_environment(name: String) -> Result<AppState, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Environment name is required".to_string());
    }
    let mut cfg = load_config()?;
    bootstrap_default_environment(&mut cfg)?;

    if cfg
        .environments
        .iter()
        .any(|e| e.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("Environment \"{name}\" already exists"));
    }

    let id = new_id();
    prepare_inactive_environment(&id)?;
    cfg.environments.push(Environment {
        id,
        name,
        created_at: now_iso(),
    });
    save_config(&cfg)?;
    state_from(cfg)
}

/// Copy the current (active) Cursor folders into an existing inactive environment.
#[tauri::command]
pub async fn copy_environment_from_current(
    app: AppHandle,
    env_id: String,
) -> Result<AppState, String> {
    let cfg = load_config()?;
    if !cfg.environments.iter().any(|e| e.id == env_id) {
        return Err("Environment not found".to_string());
    }
    if cfg.active.env_id.as_deref() == Some(env_id.as_str()) {
        return Err("Cannot copy into the current environment.".to_string());
    }
    if cursor_running() {
        return Err(
            "Quit Cursor before copying the current environment (data files may be locked)."
                .to_string(),
        );
    }

    let src_data = active_data_dir()?;
    let src_dot = active_dot_cursor_dir()?;
    let dst_data = inactive_data_dir(&env_id)?;
    let dst_dot = inactive_dot_cursor_dir(&env_id)?;
    let accounts = root_dir()?.join("accounts").join(&env_id);

    if !src_data.exists() {
        return Err(format!(
            "Nothing to copy: {} does not exist",
            src_data.display()
        ));
    }

    emit_progress(&app, 0, "Copying Application Support…");

    let app_copy = app.clone();
    let copy_id = env_id.clone();
    let copy_result = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<(), String> {
            copy_cursor_tree_with_progress(
                &app_copy,
                &src_data,
                &dst_data,
                "Copying Application Support…",
                0,
                50,
            )?;
            if src_dot.exists() {
                copy_cursor_tree_with_progress(
                    &app_copy,
                    &src_dot,
                    &dst_dot,
                    "Copying ~/.cursor…",
                    50,
                    100,
                )?;
            } else {
                std::fs::create_dir_all(&dst_dot).map_err(|e| e.to_string())?;
            }
            std::fs::create_dir_all(&accounts).map_err(|e| e.to_string())?;
            emit_progress(&app_copy, 100, "Copy complete");
            Ok(())
        })();

        if result.is_err() {
            cleanup_partial_copy(&dst_data);
            cleanup_partial_copy(&dst_dot);
            // Recreate empty skeletons so the env remains usable.
            let _ = prepare_inactive_environment(&copy_id);
        }
        result
    })
    .await
    .map_err(|e| format!("Copy task failed: {e}"))?;

    if let Err(err) = copy_result {
        emit_progress(&app, 0, "Copy failed");
        // Keep the environment listed — only the data copy failed.
        return Err(err);
    }

    let cfg = load_config()?;
    state_from(cfg)
}

#[tauri::command]
pub fn rename_environment(id: String, name: String) -> Result<AppState, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Environment name is required".to_string());
    }
    let mut cfg = load_config()?;
    if cfg
        .environments
        .iter()
        .any(|e| e.id != id && e.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("Environment \"{name}\" already exists"));
    }
    let env = cfg
        .environments
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| "Environment not found".to_string())?;
    env.name = name;
    save_config(&cfg)?;
    state_from(cfg)
}

#[tauri::command]
pub async fn delete_environment(app: AppHandle, id: String) -> Result<AppState, String> {
    let mut cfg = load_config()?;
    if !cfg.environments.iter().any(|e| e.id == id) {
        return Err("Environment not found".to_string());
    }
    if cfg.active.env_id.as_deref() == Some(id.as_str()) {
        return Err(
            "Cannot delete the current environment. Switch to another environment first."
                .to_string(),
        );
    }
    if cfg.environments.len() <= 1 {
        return Err("Cannot delete the only environment.".to_string());
    }

    // Inactive pool folders only — never touch the live Cursor / ~/.cursor names.
    let paths = vec![
        (
            inactive_data_dir(&id)?,
            "Moving Application Support to Trash…".to_string(),
        ),
        (
            inactive_dot_cursor_dir(&id)?,
            "Moving ~/.cursor to Trash…".to_string(),
        ),
        (
            root_dir()?.join("accounts").join(&id),
            "Removing saved logins…".to_string(),
        ),
    ];

    emit_progress(&app, 0, "Preparing…");
    let app_trash = app.clone();
    tauri::async_runtime::spawn_blocking(move || trash_paths_with_progress(&app_trash, &paths))
        .await
        .map_err(|e| format!("Delete task failed: {e}"))??;

    cfg.environments.retain(|e| e.id != id);
    cfg.accounts.retain(|a| a.env_id != id);
    save_config(&cfg)?;
    emit_progress(&app, 100, "Moved to Trash");
    state_from(cfg)
}

#[tauri::command]
pub fn create_account(env_id: String) -> Result<AppState, String> {
    if cursor_running() {
        return Err(
            "Cursor is still running. Wait for it to quit before adding an account.".to_string(),
        );
    }

    let mut cfg = load_config()?;
    bootstrap_default_environment(&mut cfg)?;
    if !cfg.environments.iter().any(|e| e.id == env_id) {
        return Err("Environment not found".to_string());
    }

    switch_environment(&mut cfg, &env_id)?;
    save_active_account_tokens(&cfg, &env_id)?;

    let id = new_id();
    clear_auth_keys(&state_db(&cfg, &env_id)?)?;
    save_snapshot(&env_id, &id, &AuthSnapshot::default())?;

    cfg.accounts.push(Account {
        id: id.clone(),
        env_id: env_id.clone(),
        name: "Signing in…".to_string(),
        email: None,
        updated_at: now_iso(),
        pending_login: true,
    });
    cfg.active.env_id = Some(env_id);
    cfg.active.account_id = Some(id);
    save_config(&cfg)?;

    launch_cursor(&cfg.cursor_app_path)?;
    state_from(cfg)
}

#[tauri::command]
pub fn delete_account(id: String) -> Result<AppState, String> {
    let mut cfg = load_config()?;
    let account = cfg
        .accounts
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or_else(|| "Account not found".to_string())?;

    let is_active = cfg.active.account_id.as_deref() == Some(id.as_str());
    // Only the live auth DB needs Cursor quit. Inactive accounts are just a
    // Multi Cursor snapshot + config entry.
    if is_active && cursor_running() {
        return Err(
            "Cursor is still running. Wait for it to quit before deleting the current account."
                .to_string(),
        );
    }

    delete_snapshot(&account.env_id, &account.id)?;
    cfg.accounts.retain(|a| a.id != id);

    if is_active {
        cfg.active.account_id = cfg
            .accounts
            .iter()
            .find(|a| a.env_id == account.env_id)
            .map(|a| a.id.clone());
        if let Some(next_id) = cfg.active.account_id.clone() {
            let snap = load_snapshot(&account.env_id, &next_id)?;
            write_auth_keys(&state_db(&cfg, &account.env_id)?, &snap)?;
        } else {
            clear_auth_keys(&state_db(&cfg, &account.env_id)?)?;
        }
    }

    save_config(&cfg)?;
    state_from(cfg)
}

#[tauri::command]
pub fn switch_account(env_id: String, account_id: String) -> Result<AppState, String> {
    if cursor_running() {
        return Err(
            "Cursor is still running. Wait for it to quit before switching accounts.".to_string(),
        );
    }
    let mut cfg = load_config()?;
    apply_account(&mut cfg, &env_id, &account_id)?;
    state_from(cfg)
}

#[tauri::command]
pub fn launch(env_id: String, account_id: Option<String>) -> Result<AppState, String> {
    let mut cfg = load_config()?;
    bootstrap_default_environment(&mut cfg)?;
    if !cfg.environments.iter().any(|e| e.id == env_id) {
        return Err("Environment not found".to_string());
    }

    if let Some(ref account_id) = account_id {
        let same = cfg.active.env_id.as_deref() == Some(env_id.as_str())
            && cfg.active.account_id.as_deref() == Some(account_id.as_str());
        if cursor_running() && same {
            return Err("Cursor is already running with this account.".to_string());
        }
        if cursor_running() {
            return Err(
                "Cursor is still running. Wait for it to quit before launching.".to_string(),
            );
        }
        apply_account(&mut cfg, &env_id, account_id)?;
    } else {
        if cursor_running() {
            return Err(
                "Cursor is still running. Wait for it to quit before launching.".to_string(),
            );
        }
        switch_environment(&mut cfg, &env_id)?;
        cfg.active.account_id = None;
        save_config(&cfg)?;
    }

    launch_cursor(&cfg.cursor_app_path)?;
    state_from(cfg)
}

/// Ask Cursor to quit without waiting — UI polls `is_cursor_running` afterward.
#[tauri::command]
pub fn quit_cursor_cmd() -> Result<(), String> {
    request_quit_cursor()
}

#[tauri::command]
pub fn force_quit_cursor_cmd() -> Result<(), String> {
    force_quit_cursor()
}

#[tauri::command]
pub fn is_cursor_running() -> bool {
    cursor_running()
}
