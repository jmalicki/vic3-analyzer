//! Fat desktop binary: one artifact for GUI and stdio MCP (`docs/mcp.md`).
//!
//! Mode is chosen from argv **before** Tauri `run` so MCP never creates a
//! WebView. Both modes share on-disk [`vic3_catalog::AppConfig`] / defs cache;
//! MCP keeps its own in-process SQL session (not shared RAM with the GUI).
//! WebView/runtime libraries may still map at process start — see
//! `docs/mcp.md` / `docs/desktop.md`.
//!
//! # Shipping story (Wave 4b)
//!
//! - Default / `gui` → [`vic3_analyzer_lib::run`] (Tauri WebView window).
//! - `mcp` → [`vic3_mcp::run`] only — **early argv branch** before any
//!   `tauri::Builder::…run()`. That control flow is what guarantees no window.
//!
//! WebView native libraries (WKWebView / WebKitGTK / WebView2) may still be
//! **mapped at process start** because this binary links Tauri. That does not
//! open a window; it is acceptable for v1. A second MCP-only artifact is
//! deferred unless headless CI forces a feature-split.

use std::process::ExitCode;
use vic3_analyzer_lib::Mode;

fn main() -> ExitCode {
    // Parse argv before touching Tauri so `mcp` never enters the GUI event loop.
    match Mode::from_args(std::env::args()) {
        Mode::Mcp => vic3_mcp::run(),
        Mode::Gui => {
            vic3_analyzer_lib::run();
            ExitCode::SUCCESS
        }
    }
}
