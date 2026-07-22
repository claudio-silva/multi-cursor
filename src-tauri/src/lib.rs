mod auth;
mod commands;
mod copy_progress;
mod cursor;
mod paths;
mod window_frame;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            window_frame::attach(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_state,
            commands::create_environment,
            commands::copy_environment_from_current,
            commands::environment_disk_usage,
            commands::rename_environment,
            commands::delete_environment,
            commands::create_account,
            commands::delete_account,
            commands::switch_account,
            commands::launch,
            commands::quit_cursor_cmd,
            commands::force_quit_cursor_cmd,
            commands::is_cursor_running,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            window_frame::handle_run_event(app, &event);
        });
}
