//! OpenSymphony Tauri desktop entry point.

use std::process;
use tauri::Manager;

mod commands;
mod types;
mod settings;
mod keychain;
mod actions;

fn main() {
    if let Err(e) = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            if let Some(_window) = app.get_webview_window("main") {
                // Window exists; future setup hooks can attach here.
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_setting,
            settings::set_setting,
            keychain::get_credential,
            keychain::set_credential,
            keychain::delete_credential,
            keychain::credential_status,
            actions::open_file,
            actions::open_folder,
            actions::open_repository_folder,
            actions::reveal_workspace,
            actions::copy_to_clipboard,
            actions::open_linear_link,
            actions::notify,
            commands::daemon_status,
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("Tauri runtime error: {e}");
        process::exit(1);
    }
}
