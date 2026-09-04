use serde::{Deserialize, Serialize};

use crate::paths::{ensure_layout, config_path, default_cursor_app_path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub env_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub pending_login: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSelection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub environments: Vec<Environment>,
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub active: ActiveSelection,
    #[serde(default = "default_cursor_app_path")]
    pub cursor_app_path: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            environments: Vec::new(),
            accounts: Vec::new(),
            active: ActiveSelection::default(),
            cursor_app_path: default_cursor_app_path(),
        }
    }
}

pub fn load_config() -> Result<AppConfig, String> {
    ensure_layout()?;
    let path = config_path()?;
    if !path.exists() {
        let cfg = AppConfig::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("Invalid config.json: {e}"))
}

pub fn save_config(cfg: &AppConfig) -> Result<(), String> {
    ensure_layout()?;
    let path = config_path()?;
    let raw = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, raw).map_err(|e| e.to_string())
}
