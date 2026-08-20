//! Desktop binary: GUI by default; `mcp` argv runs stdio MCP without a window.
//!
//! Mode is chosen from argv **before** Tauri `run` so MCP never creates a
//! WebView. Both modes share on-disk [`vic3_catalog::AppConfig`]; MCP keeps its
//! own in-process SQL session.

use std::process::ExitCode;
use vic3_analyzer_lib::Mode;

fn main() -> ExitCode {
    match Mode::from_args(std::env::args()) {
        Mode::Mcp => vic3_mcp::run(),
        Mode::Gui => {
            vic3_analyzer_lib::run();
            ExitCode::SUCCESS
        }
    }
}
