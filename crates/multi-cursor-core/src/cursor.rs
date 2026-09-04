use std::process::{Command, Stdio};

/// True when the Cursor IDE main process is running.
///
/// On macOS uses `killall -s` matching the exact process name `Cursor`.
/// Helper processes are named `Cursor Helper*` and are not matched.
/// On Linux there is typically no IDE; returns false.
pub fn cursor_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("killall")
            .args(["-s", "Cursor"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.lines().any(|l| l.contains("kill "))
            }
            _ => false,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Refuse mutating Cursor auth while the IDE is open (it can rewrite Keychain /
/// state.vscdb and undo or scramble the switch).
pub fn require_cursor_idle() -> Result<(), String> {
    if cursor_running() {
        return Err(
            "Cursor IDE is running. Quit Cursor completely, then try again."
                .to_string(),
        );
    }
    Ok(())
}
