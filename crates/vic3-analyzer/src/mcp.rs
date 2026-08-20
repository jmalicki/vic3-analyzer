//! MCP stdio server stub (Wave 2c).
//!
//! Real rmcp wiring is deferred. This path must never call Tauri `run` /
//! create a window. Protocol bytes belong on stdout later; logs on stderr.

use std::io::{self, Write};
use std::process::ExitCode;

/// Run the MCP stub and return a process exit code.
pub fn run_stub() -> ExitCode {
    let mut err = io::stderr().lock();
    let _ = writeln!(
        err,
        "vic3-analyzer mcp: stub only (stdio MCP not implemented yet)"
    );
    ExitCode::SUCCESS
}
