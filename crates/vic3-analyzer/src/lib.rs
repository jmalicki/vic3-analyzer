//! Tauri desktop library entry (shared with the `vic3-analyzer` binary).

pub mod mode;

pub use mode::Mode;

/// Placeholder invoke proving `vic3-api` is linked into the desktop binary.
#[tauri::command]
fn api_ping() -> Result<&'static str, String> {
    // Touch the API error type so the crate stays linked without loading saves.
    let _ = vic3_api::ApiError::NoLoadedAnalysis;
    Ok("pong")
}

/// Start the Tauri GUI (WebView). Call only for [`Mode::Gui`].
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![api_ping])
        .run(tauri::generate_context!())
        .expect("error while running vic3-analyzer");
}
