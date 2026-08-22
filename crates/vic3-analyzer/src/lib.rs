//! Tauri desktop library entry (shared with the `vic3-analyzer` binary).
//!
//! # Modes
//!
//! The binary parses argv via [`Mode`] **before** calling [`run`]:
//!
//! | Argv | Behavior |
//! | --- | --- |
//! | (none) / `gui` | [`run`] — WebView companion UI |
//! | `mcp` | `vic3_mcp::run()` — stdio MCP, no window |
//!
//! # Fat binary / WebView caveat
//!
//! One artifact links Tauri + `vic3-mcp`. MCP skips `tauri::Builder::run`, so no
//! window opens; WebView runtimes may still load at process start (v1 accepted).
//!
//! # Stack
//!
//! ```text
//! vic3-catalog (AppConfig, stubs)
//!        → CompanionSession / McpRuntime
//!        → vic3-sql::SqlEngine (use_save, query)
//!        → Tauri invokes  |  MCP tools
//! ```
//!
//! Invokes take filename stubs / config strings in and return JSON — never
//! `Vec<u8>` of saves across the WebView boundary. See `docs/desktop.md`.

pub mod commands;
pub mod dto;
pub mod mode;
pub mod session;
pub mod watch;

pub use mode::Mode;

/// Start the Tauri GUI (WebView). Call only for [`Mode::Gui`].
///
/// Registers companion invoke handlers and save-dir watch. Panics if Tauri
/// context generation / event loop fails (same as typical Tauri apps).
/// Must never be invoked for [`Mode::Mcp`]: the binary’s early argv branch in
/// `main` routes MCP to `vic3_mcp::run` so this function does not run.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
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
        ]);

    // Docs / WDIO only — never enable the `webdriver` feature in player release builds.
    #[cfg(feature = "webdriver")]
    {
        builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    }

    builder
        .run(tauri::generate_context!())
        .expect("error while running vic3-analyzer");
}
