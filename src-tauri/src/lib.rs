mod about;
mod auth;
mod commands;
mod copy_progress;
mod cursor;
mod paths;
mod updates;
mod window_frame;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter};

#[cfg(not(target_os = "macos"))]
use tauri::menu::AboutMetadata;

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let window_menu = Submenu::with_id_and_items(
        app,
        "window",
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let about_item = MenuItem::with_id(
        app,
        "about",
        format!("About {}", about::APP_NAME),
        true,
        None::<&str>,
    )?;

    let check_updates_item = MenuItem::with_id(
        app,
        "check-for-updates",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;

    #[cfg(not(target_os = "macos"))]
    let about_item = {
        let version = app.package_info().version.to_string();
        PredefinedMenuItem::about(
            app,
            None,
            Some(AboutMetadata {
                name: Some(about::APP_NAME.into()),
                version: Some(version),
                credits: Some(about::credits_text()),
                website: Some(about::REPO_URL.into()),
                website_label: Some(about::REPO_URL.into()),
                authors: Some(vec!["Cláudio Silva".into()]),
                comments: Some(
                    "Switch between Cursor accounts and isolated environments.".into(),
                ),
                ..Default::default()
            }),
        )?
    };

    let help_menu = Submenu::with_id_and_items(
        app,
        "help",
        "Help",
        true,
        &[
            #[cfg(not(target_os = "macos"))]
            &about_item,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(not(target_os = "macos"))]
            &check_updates_item,
        ],
    )?;

    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &Submenu::with_items(
                app,
                about::APP_NAME,
                true,
                &[
                    &about_item,
                    &check_updates_item,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            #[cfg(not(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            )))]
            &Submenu::with_items(
                app,
                "File",
                true,
                &[
                    &PredefinedMenuItem::close_window(app, None)?,
                    #[cfg(not(target_os = "macos"))]
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            #[cfg(target_os = "macos")]
            &Submenu::with_items(
                app,
                "View",
                true,
                &[&PredefinedMenuItem::fullscreen(app, None)?],
            )?,
            &window_menu,
            &help_menu,
        ],
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .menu(build_menu)
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "about" => {
                    #[cfg(target_os = "macos")]
                    about::show(&app.package_info().version.to_string());
                }
                "check-for-updates" => {
                    let _ = app.emit("check-for-updates", ());
                }
                _ => {}
            }
        })
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
            updates::check_for_updates,
            updates::open_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            window_frame::handle_run_event(app, &event);
        });
}
