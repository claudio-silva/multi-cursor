//! Window frame persistence.
//!
//! On macOS, Tauri/tao's physical↔logical conversion breaks across mixed-scale
//! displays (Retina laptop + 1× externals). AppKit's frame autosave speaks the
//! same coordinate space as the Dock and handles multi-monitor correctly.

use tauri::{AppHandle, Manager, RunEvent, WebviewWindow};

const AUTOSAVE_NAME: &str = "MultiCursorMainWindow";

#[cfg(target_os = "macos")]
fn apply_macos_frame_autosave(window: &WebviewWindow) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::NSString;

    let ptr = window
        .ns_window()
        .map_err(|e| format!("ns_window: {e}"))? as *mut NSWindow;
    if ptr.is_null() {
        return Err("ns_window was null".to_string());
    }

    // SAFETY: Tauri's ns_window is a valid NSWindow* for this WebviewWindow's lifetime.
    let ns_window = unsafe { Retained::retain(ptr) }
        .ok_or_else(|| "Failed to retain NSWindow".to_string())?;
    let name = NSString::from_str(AUTOSAVE_NAME);

    // Restores a previously saved frame if one exists, then enables autosave
    // so AppKit keeps the display + position updated as the user moves the window.
    let _ = ns_window.setFrameUsingName(&name);
    let _ = ns_window.setFrameAutosaveName(&name);

    Ok(())
}

#[cfg(target_os = "macos")]
fn save_macos_frame_now(window: &WebviewWindow) {
    use objc2::rc::Retained;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::NSString;

    let Ok(ptr) = window.ns_window() else {
        return;
    };
    let ptr = ptr as *mut NSWindow;
    if ptr.is_null() {
        return;
    }
    let Some(ns_window) = (unsafe { Retained::retain(ptr) }) else {
        return;
    };
    let name = NSString::from_str(AUTOSAVE_NAME);
    let _ = ns_window.setFrameAutosaveName(&name);
    ns_window.saveFrameUsingName(&name);
}

/// Attach frame restore + show. Prefer AppKit autosave on macOS.
pub fn attach(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    #[cfg(target_os = "macos")]
    {
        if let Err(err) = apply_macos_frame_autosave(&window) {
            eprintln!("window frame autosave: {err}");
        }
        let window_for_events = window.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                save_macos_frame_now(&window_for_events);
            }
        });
    }

    let _ = window.show();
    let _ = window.set_focus();
}

pub fn handle_run_event(app: &AppHandle, event: &RunEvent) {
    #[cfg(target_os = "macos")]
    if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
        if let Some(window) = app.get_webview_window("main") {
            save_macos_frame_now(&window);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, event);
    }
}
