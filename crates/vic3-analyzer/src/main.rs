//! Desktop binary: GUI by default, `mcp` argv runs stdio MCP without a window.

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
