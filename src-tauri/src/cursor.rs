use std::path::Path;
use std::process::{Command, Stdio};

/// True when the Cursor IDE main process is running.
///
/// Uses `killall -s` (dry-run) matching the exact process name `Cursor`.
/// Prefer this over `pgrep -f …/MacOS/Cursor`: on recent macOS, `pgrep` often
/// cannot see the Electron main process even when Cursor is running, while
/// `killall` still can. Helper processes are named `Cursor Helper*` and are
/// not matched. Multi Cursor's own binary is `multi-cursor`, so it is safe.
pub fn cursor_running() -> bool {
    let output = Command::new("killall")
        .args(["-s", "Cursor"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // Dry-run lines look like: `kill -term 12345`
            text.lines().any(|l| l.contains("kill "))
        }
        _ => false,
    }
}

/// Ask Cursor to quit; returns immediately without waiting for exit.
pub fn request_quit_cursor() -> Result<(), String> {
    let _ = Command::new("osascript")
        .args(["-e", "tell application \"Cursor\" to quit"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    Ok(())
}

/// Force-kill Cursor processes (non-blocking spawn).
pub fn force_quit_cursor() -> Result<(), String> {
    let _ = Command::new("killall")
        .arg("Cursor")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    Ok(())
}

/// Launch Cursor with its default paths (Application Support/Cursor + ~/.cursor).
pub fn launch_cursor(app_path: &str) -> Result<(), String> {
    if !Path::new(app_path).exists() {
        return Err(format!("Cursor app not found at {app_path}"));
    }
    // `open` returns as soon as Launch Services accepts the request.
    let status = Command::new("open")
        .arg(app_path)
        .status()
        .map_err(|e| format!("Failed to launch Cursor: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("open failed to launch Cursor".to_string())
    }
}
