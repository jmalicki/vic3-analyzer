//! Stdio MCP server for Vic3 Analyzer (`docs/mcp.md`).
//!
//! Invoked as `vic3-analyzer mcp`: JSON-RPC on stdout, logs on stderr, no
//! window. Shares [`vic3_catalog::AppConfig`] and [`vic3_sql::SqlEngine`] with
//! the desktop stack (separate process / RAM session in v1).

mod error;
mod format;
mod runtime;
mod server;

pub use runtime::{McpBootstrapError, McpRuntime};
pub use server::Vic3McpServer;

use std::process::ExitCode;

/// Binary entry for `vic3-analyzer mcp`: shared config → stdio MCP → exit code.
///
/// Never opens a Tauri window. Protocol bytes stay on stdout; tracing goes to
/// stderr only. Session RAM is process-local (not shared with a concurrent GUI).
pub fn run() -> ExitCode {
    init_stderr_logging();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            tracing::error!(%err, "failed to build tokio runtime");
            return ExitCode::FAILURE;
        }
    };

    match rt.block_on(run_async()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "mcp server exited with error");
            ExitCode::FAILURE
        }
    }
}

async fn run_async() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = McpRuntime::open(None).await?;
    tracing::info!(
        saves = runtime.save_count(),
        config = %runtime.config_path().display(),
        "vic3-analyzer mcp ready"
    );
    Vic3McpServer::serve_stdio(runtime).await?;
    Ok(())
}

/// Install a stderr-only subscriber so MCP JSON-RPC on stdout stays clean.
fn init_stderr_logging() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}
