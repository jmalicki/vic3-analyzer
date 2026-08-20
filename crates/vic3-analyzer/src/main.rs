//! Desktop binary: GUI by default, `mcp` argv stub without a window.

mod mcp;

use std::process::ExitCode;
use vic3_analyzer_lib::Mode;

fn main() -> ExitCode {
    match Mode::from_args(std::env::args()) {
        Mode::Mcp => mcp::run_stub(),
        Mode::Gui => {
            vic3_analyzer_lib::run();
            ExitCode::SUCCESS
        }
    }
}
