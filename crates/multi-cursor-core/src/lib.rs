//! Shared Multi Cursor logic for account snapshots and Cursor Agent CLI auth.

pub mod accounts;
pub mod agent_auth;
pub mod config;
pub mod cursor;
pub mod paths;
pub mod snapshot;

#[cfg(target_os = "macos")]
pub mod ide_auth;

pub use accounts::*;
pub use agent_auth::{clear_cli_auth, read_live_agent_snapshot, sync_cli_auth_from_snapshot};
pub use config::*;
pub use cursor::{cursor_running, require_cursor_idle};
pub use paths::{
    account_snapshot_path, active_dot_cursor_dir, ensure_layout, home_dir, new_id, now_iso,
    root_dir, AUTH_KEYS,
};
pub use snapshot::*;
