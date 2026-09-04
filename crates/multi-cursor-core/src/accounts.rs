//! High-level account operations for the CLI (and shared helpers).

use serde::{Deserialize, Serialize};

use crate::agent_auth::{
    clear_cli_auth, read_live_agent_snapshot, sync_cli_auth_from_snapshot,
};
use crate::config::{save_config, Account, AppConfig, Environment, load_config};
use crate::cursor::require_cursor_idle;
use crate::paths::{ensure_layout, new_id, now_iso, root_dir};
use crate::snapshot::{
    delete_snapshot, display_name_from_snapshot, email_from_snapshot, has_login_tokens,
    load_snapshot, save_snapshot, AuthSnapshot,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountExport {
    pub version: u32,
    pub exported_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub snapshot: AuthSnapshot,
}

/// Ensure a Default environment exists and adopt the live Agent login when empty.
pub fn bootstrap_cli_state() -> Result<AppConfig, String> {
    ensure_layout()?;
    let mut cfg = load_config()?;

    if cfg.environments.is_empty() {
        let id = new_id();
        cfg.environments.push(Environment {
            id: id.clone(),
            name: "Default".to_string(),
            created_at: now_iso(),
        });
        cfg.active.env_id = Some(id);
        save_config(&cfg)?;
    }

    if cfg.active.env_id.is_none() {
        cfg.active.env_id = cfg.environments.first().map(|e| e.id.clone());
        save_config(&cfg)?;
    }

    let Some(env_id) = cfg.active.env_id.clone() else {
        return Ok(cfg);
    };

    std::fs::create_dir_all(root_dir()?.join("accounts").join(&env_id))
        .map_err(|e| e.to_string())?;

    if !cfg.accounts.iter().any(|a| a.env_id == env_id) {
        if let Ok(snap) = read_live_agent_snapshot() {
            if has_login_tokens(&snap) {
                let account_id = new_id();
                save_snapshot(&env_id, &account_id, &snap)?;
                let email = email_from_snapshot(&snap);
                let name = display_name_from_snapshot(&snap)
                    .unwrap_or_else(|| "Signed-in account".to_string());
                cfg.accounts.push(Account {
                    id: account_id.clone(),
                    env_id: env_id.clone(),
                    name,
                    email,
                    updated_at: now_iso(),
                    pending_login: false,
                });
                cfg.active.account_id = Some(account_id);
                save_config(&cfg)?;
            }
        }
    }

    Ok(cfg)
}

pub fn find_account<'a>(cfg: &'a AppConfig, query: &str) -> Result<&'a Account, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("Account query is empty".to_string());
    }

    let matches: Vec<&Account> = cfg
        .accounts
        .iter()
        .filter(|a| {
            a.id == q
                || a.name.eq_ignore_ascii_case(q)
                || a.email
                    .as_ref()
                    .map(|e| e.eq_ignore_ascii_case(q))
                    .unwrap_or(false)
                || a.email
                    .as_ref()
                    .map(|e| e.to_lowercase().contains(&q.to_lowercase()))
                    .unwrap_or(false)
                || a.name.to_lowercase().contains(&q.to_lowercase())
        })
        .collect();

    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!("No account matches \"{q}\"")),
        many => {
            let list = many
                .iter()
                .map(|a| {
                    format!(
                        "{} ({})",
                        a.name,
                        a.email.as_deref().unwrap_or(&a.id)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!("Ambiguous account \"{q}\": {list}"))
        }
    }
}

pub fn switch_account(query: &str) -> Result<Account, String> {
    require_cursor_idle()?;

    let mut cfg = bootstrap_cli_state()?;
    let account = find_account(&cfg, query)?.clone();
    if account.pending_login {
        return Err("Account is still pending login".to_string());
    }

    let snap = load_snapshot(&account.env_id, &account.id)?;
    if !has_login_tokens(&snap) {
        return Err("Account snapshot has no login tokens".to_string());
    }

    // Persist the current live login back onto the previously active account
    // only when the live identity still matches that account.
    if let Some(prev_id) = cfg.active.account_id.clone() {
        if prev_id != account.id {
            if let Some(prev) = cfg.accounts.iter().find(|a| a.id == prev_id) {
                if !prev.pending_login {
                    if let Ok(live) = read_live_agent_snapshot() {
                        if has_login_tokens(&live) && live_matches_account(&live, prev) {
                            let _ = save_snapshot(&prev.env_id, &prev.id, &live);
                        }
                    }
                }
            }
        }
    }

    sync_cli_auth_from_snapshot(&snap)?;

    #[cfg(target_os = "macos")]
    {
        if let Ok(db) = crate::paths::env_state_db(&account.env_id, Some(account.env_id.as_str()))
        {
            if db.exists() || db.parent().map(|p| p.exists()).unwrap_or(false) {
                crate::ide_auth::write_auth_keys(&db, &snap)?;
            }
        }
    }

    cfg.active.env_id = Some(account.env_id.clone());
    cfg.active.account_id = Some(account.id.clone());
    save_config(&cfg)?;
    Ok(account)
}

fn live_matches_account(live: &AuthSnapshot, account: &Account) -> bool {
    match (email_from_snapshot(live), account.email.as_deref()) {
        (Some(live_email), Some(account_email)) => live_email.eq_ignore_ascii_case(account_email),
        _ => false,
    }
}

pub fn capture_current(name: Option<String>) -> Result<Account, String> {
    require_cursor_idle()?;

    let mut cfg = bootstrap_cli_state()?;
    let env_id = cfg
        .active
        .env_id
        .clone()
        .ok_or_else(|| "No active environment".to_string())?;

    let snap = read_live_agent_snapshot()?;
    if !has_login_tokens(&snap) {
        return Err("No Cursor Agent login found to capture".to_string());
    }

    let email = email_from_snapshot(&snap);
    let display = display_name_from_snapshot(&snap);

    // Update existing account with same email if present.
    if let Some(email_ref) = email.as_deref() {
        if let Some(existing) = cfg
            .accounts
            .iter_mut()
            .find(|a| a.env_id == env_id && a.email.as_deref() == Some(email_ref))
        {
            save_snapshot(&env_id, &existing.id, &snap)?;
            existing.updated_at = now_iso();
            existing.pending_login = false;
            if let Some(n) = name {
                existing.name = n;
            } else if let Some(d) = display {
                existing.name = d;
            }
            cfg.active.account_id = Some(existing.id.clone());
            let out = existing.clone();
            save_config(&cfg)?;
            return Ok(out);
        }
    }

    let account_id = new_id();
    save_snapshot(&env_id, &account_id, &snap)?;
    let account = Account {
        id: account_id.clone(),
        env_id,
        name: name
            .or(display)
            .or_else(|| email.clone())
            .unwrap_or_else(|| "Captured account".to_string()),
        email,
        updated_at: now_iso(),
        pending_login: false,
    };
    cfg.accounts.push(account.clone());
    cfg.active.account_id = Some(account_id);
    save_config(&cfg)?;
    Ok(account)
}

pub fn export_account(query: &str) -> Result<AccountExport, String> {
    let cfg = bootstrap_cli_state()?;
    let account = find_account(&cfg, query)?;
    let snap = load_snapshot(&account.env_id, &account.id)?;
    if !has_login_tokens(&snap) {
        return Err("Account snapshot has no login tokens".to_string());
    }
    Ok(AccountExport {
        version: 1,
        exported_at: now_iso(),
        email: account.email.clone().or_else(|| email_from_snapshot(&snap)),
        name: Some(account.name.clone()),
        snapshot: snap,
    })
}

pub fn import_account(envelope: AccountExport) -> Result<Account, String> {
    if envelope.version != 1 {
        return Err(format!(
            "Unsupported export version {}",
            envelope.version
        ));
    }
    if !has_login_tokens(&envelope.snapshot) {
        return Err("Import envelope has no login tokens".to_string());
    }

    let mut cfg = bootstrap_cli_state()?;
    let env_id = cfg
        .active
        .env_id
        .clone()
        .ok_or_else(|| "No active environment".to_string())?;

    let email = envelope
        .email
        .clone()
        .or_else(|| email_from_snapshot(&envelope.snapshot));

    if let Some(email_ref) = email.as_deref() {
        if let Some(existing) = cfg
            .accounts
            .iter_mut()
            .find(|a| a.env_id == env_id && a.email.as_deref() == Some(email_ref))
        {
            save_snapshot(&env_id, &existing.id, &envelope.snapshot)?;
            existing.updated_at = now_iso();
            existing.pending_login = false;
            if let Some(n) = envelope.name {
                existing.name = n;
            }
            let out = existing.clone();
            save_config(&cfg)?;
            return Ok(out);
        }
    }

    let account_id = new_id();
    save_snapshot(&env_id, &account_id, &envelope.snapshot)?;
    let account = Account {
        id: account_id,
        env_id,
        name: envelope
            .name
            .or_else(|| display_name_from_snapshot(&envelope.snapshot))
            .or_else(|| email.clone())
            .unwrap_or_else(|| "Imported account".to_string()),
        email,
        updated_at: now_iso(),
        pending_login: false,
    };
    cfg.accounts.push(account.clone());
    save_config(&cfg)?;
    Ok(account)
}

pub fn remove_account(query: &str) -> Result<Account, String> {
    let mut cfg = bootstrap_cli_state()?;
    let account = find_account(&cfg, query)?.clone();
    let was_active = cfg.active.account_id.as_deref() == Some(account.id.as_str());
    if was_active {
        require_cursor_idle()?;
    }

    delete_snapshot(&account.env_id, &account.id)?;
    cfg.accounts.retain(|a| a.id != account.id);

    if was_active {
        cfg.active.account_id = cfg
            .accounts
            .iter()
            .find(|a| a.env_id == account.env_id)
            .map(|a| a.id.clone());
        if let Some(next_id) = cfg.active.account_id.clone() {
            let snap = load_snapshot(&account.env_id, &next_id)?;
            sync_cli_auth_from_snapshot(&snap)?;
        } else {
            clear_cli_auth()?;
        }
    }

    save_config(&cfg)?;
    Ok(account)
}

pub fn agent_about_email() -> Option<String> {
    let output = std::process::Command::new("cursor-agent")
        .arg("about")
        .output()
        .ok()
        .or_else(|| {
            std::process::Command::new("agent")
                .arg("about")
                .output()
                .ok()
        })?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("User Email") {
            let email = rest.trim().trim_start_matches(':').trim();
            if !email.is_empty() {
                return Some(email.to_string());
            }
        }
    }
    None
}
