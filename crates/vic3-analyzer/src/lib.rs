//! Tauri desktop library entry (shared with the `vic3-analyzer` binary).

pub mod commands;
pub mod dto;
pub mod mode;
pub mod session;
pub mod watch;

pub use mode::Mode;

/// Start the Tauri GUI (WebView). Call only for [`Mode::Gui`].
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(commands::setup_app)
        .invoke_handler(tauri::generate_handler![
            commands::api_ping,
            commands::get_config,
            commands::save_config,
            commands::reset_config,
            commands::list_saves,
            commands::get_dashboard,
            commands::detection_hints,
            commands::use_save,
            commands::loaded_prices,
            commands::loaded_alerts,
            commands::loaded_gaps,
            commands::sql_query,
            commands::sql_docs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running vic3-analyzer");
}
