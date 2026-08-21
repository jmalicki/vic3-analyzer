//! Stdio MCP server for Vic3 Analyzer (`docs/mcp.md`).
//!
//! Invoked as `vic3-analyzer mcp` via the fat binary’s early argv branch:
//! JSON-RPC on **stdout**, logs on **stderr**, **no Tauri window**. WebView
//! native libraries may still load because the binary links Tauri (documented
//! v1 caveat). Shares [`vic3_catalog::DesktopConfig`] /
//! [`vic3_api::ensure_defs_blob`] with the GUI; SQL session state stays
//! process-local.
//!
//! # Why a fat binary
//!
//! v1 ships one `vic3-analyzer` artifact that links Tauri + MCP. Early argv
//! (`gui` default / `mcp`) runs before `tauri::Builder::run`, so MCP never opens
//! a WebView — but WebView/runtime libraries may still map at process start. A
//! second headless artifact is deferred.
//!
//! # Tools / resources / prompts
//!
//! | Kind | Names |
//! | --- | --- |
//! | Tools | `query`, `use_save`, `refresh_catalog`, `explain`, `preview_delta` |
//! | Resources | `vic3://schema`, `vic3://saves`, `vic3://session`, `vic3://docs/*` |
//! | Prompts | `investigate_shortages`, `compare_latest_autosave`, `military_readiness`, `what_is_loaded`, `plan_research` |
//!
//! Agent flow: catalog → `use_save` tool → read-only SQL (`docs/sql.md`).
//!
//! # See also
//!
//! - [`McpRuntime`] — config + locked [`vic3_sql::SqlEngine`]
//! - [`Vic3McpServer`] — rmcp `ServerHandler`
//! - `docs/mcp.md`, `docs/desktop.md`

mod error;
mod format;
mod runtime;
mod server;

pub use runtime::{McpBootstrapError, McpRuntime};
pub use server::Vic3McpServer;

use std::process::ExitCode;

/// Binary entry for `vic3-analyzer mcp`: shared config → stdio MCP → exit code.
///
/// Never opens a Tauri window (callers must not invoke `vic3_analyzer_lib::run`
/// on this path). Protocol bytes stay on stdout; tracing goes to stderr only.
/// Session RAM is process-local (not shared with a concurrent GUI).
///
/// # Errors
///
/// Failures during bootstrap or serve are logged to stderr; the process then
/// returns [`ExitCode::FAILURE`]. This function itself always returns an exit
/// code (it does not panic on MCP errors).
pub fn run() -> ExitCode {
    init_stderr_logging();
    // Emit before catalog/defs work so headless smoke can distinguish “starting”
    // from a multi-minute first-time defs build under a real game_dir.
    tracing::info!("vic3-analyzer mcp starting (stdio; no window)");

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
    let catalog_saves = runtime
        .catalog_entries()
        .await
        .map(|e| e.len())
        .unwrap_or(0);
    tracing::info!(
        save_dirs = runtime.save_dir_count(),
        catalog_saves,
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
