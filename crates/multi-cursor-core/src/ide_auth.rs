//! Read/write Cursor IDE `state.vscdb` auth keys (macOS only).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags};
use uuid::Uuid;

use crate::paths::AUTH_KEYS;
use crate::snapshot::AuthSnapshot;

fn row_to_string(row: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
    match row.get::<_, String>(0) {
        Ok(s) => Ok(s),
        Err(_) => {
            let bytes: Vec<u8> = row.get(0)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
    }
}

fn read_auth_keys_from_conn(conn: &Connection) -> Result<AuthSnapshot, String> {
    let mut snap = AuthSnapshot::default();
    for key in AUTH_KEYS {
        let value: Result<String, rusqlite::Error> = conn.query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            params![key],
            row_to_string,
        );
        match value {
            Ok(v) => {
                snap.keys.insert((*key).to_string(), v);
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => return Err(format!("Read key {key}: {e}")),
        }
    }
    Ok(snap)
}

fn copy_db_tree(db_path: &Path, dest_db: &Path) -> Result<(), String> {
    if let Some(parent) = dest_db.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(db_path, dest_db).map_err(|e| format!("Copy state.vscdb: {e}"))?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", db_path.display()));
        if side.exists() {
            let dest = PathBuf::from(format!("{}{suffix}", dest_db.display()));
            let _ = fs::copy(&side, &dest);
        }
    }
    Ok(())
}

pub fn read_auth_keys(db_path: &Path) -> Result<AuthSnapshot, String> {
    if !db_path.exists() {
        return Ok(AuthSnapshot::default());
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;

    if let Ok(conn) = Connection::open_with_flags(db_path, flags) {
        let _ = conn.busy_timeout(Duration::from_millis(1000));
        if let Ok(snap) = read_auth_keys_from_conn(&conn) {
            return Ok(snap);
        }
    }

    let tmp_dir = std::env::temp_dir().join(format!("multi-cursor-{}", Uuid::new_v4()));
    let tmp_db = tmp_dir.join("state.vscdb");
    copy_db_tree(db_path, &tmp_db)?;
    let snap = {
        let conn = Connection::open_with_flags(&tmp_db, flags)
            .map_err(|e| format!("Open copied state.vscdb: {e}"))?;
        let _ = conn.busy_timeout(Duration::from_millis(1000));
        read_auth_keys_from_conn(&conn)?
    };
    let _ = fs::remove_dir_all(&tmp_dir);
    Ok(snap)
}

fn ensure_item_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ItemTable (
            key TEXT UNIQUE ON CONFLICT REPLACE,
            value BLOB
        );",
    )
    .map_err(|e| format!("Create ItemTable: {e}"))
}

pub fn write_auth_keys(db_path: &Path, snap: &AuthSnapshot) -> Result<(), String> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(db_path).map_err(|e| format!("Open state.vscdb: {e}"))?;
    ensure_item_table(&conn)?;

    for key in AUTH_KEYS {
        conn.execute("DELETE FROM ItemTable WHERE key = ?1", params![key])
            .map_err(|e| format!("Delete key {key}: {e}"))?;
    }
    for (key, value) in &snap.keys {
        if !AUTH_KEYS.contains(&key.as_str()) {
            continue;
        }
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| format!("Write key {key}: {e}"))?;
    }
    Ok(())
}

pub fn clear_auth_keys(db_path: &Path) -> Result<(), String> {
    write_auth_keys(db_path, &AuthSnapshot::default())
}
