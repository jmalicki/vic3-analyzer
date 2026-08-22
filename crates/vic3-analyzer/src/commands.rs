//! Tauri invoke handlers for the companion UI.
//!
//! Commands take stubs / config strings and return JSON DTOs. Absolute save
//! paths stay inside [`CompanionSession`](crate::session::CompanionSession).
//! Capability allowlist: `capabilities/default.json` + `allow-companion`.
//!
//! | Command | Role |
//! | --- | --- |
//! | `get_config` / `save_config` / `reset_config` | Settings ↔ [`vic3_catalog::AppConfig`] |
//! | `list_saves` / `get_dashboard` / `detection_hints` | Catalog + status |
//! | `use_save` | Stub → SQL bind + analysis session |
//! | `loaded_prices` / `loaded_alerts` / `loaded_gaps` | Session analysis JSON |
//! | `sql_query` / `sql_docs` | Advanced Query (shared shape with MCP) |
//! | `api_ping` | Smoke link to `vic3-api` |

use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::dto::{ConfigDto, DashboardDto, SaveStubDto};
use crate::session::{AppState, CompanionSession, SqlDocsDto};
use crate::watch::{self, WatchHandle};

/// Placeholder invoke proving `vic3-api` is linked into the desktop binary.
///
/// # Errors
///
/// Never fails today; `Result` kept for invoke shape consistency.
#[tauri::command]
pub fn api_ping() -> Result<&'static str, String> {
    let _ = vic3_api::ApiError::NoLoadedAnalysis;
    Ok("pong")
}

/// Return the current shared config DTO (paths as strings).
///
/// # Errors
///
/// `"state lock poisoned"` if the session mutex is poisoned.
#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<ConfigDto, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    Ok(session.config_dto())
}

/// Persist Settings and restart the save-dir watcher.
///
/// # Arguments
///
/// * `config` — DTO from the WebView; `config_path` on the client is ignored for writes.
///
/// # Errors
///
/// Lock poison, catalog/config I/O, or invalid paths from [`CompanionSession::apply_config`].
#[tauri::command]
pub fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: ConfigDto,
) -> Result<ConfigDto, String> {
    let dto = {
        let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
        session.apply_config(config)?
    };
    restart_watch(&app, &state)?;
    Ok(dto)
}

/// Clear overrides, re-run auto-detect, persist, restart watch.
///
/// # Errors
///
/// Lock poison or config write / catalog refresh failure.
#[tauri::command]
pub fn reset_config(app: AppHandle, state: State<'_, AppState>) -> Result<ConfigDto, String> {
    let dto = {
        let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
        session.reset_to_auto_detect()?
    };
    restart_watch(&app, &state)?;
    Ok(dto)
}

/// Rescan allowlisted roots; return stub rows (no absolute paths).
///
/// # Errors
///
/// Lock poison or catalog I/O.
#[tauri::command]
pub fn list_saves(state: State<'_, AppState>) -> Result<Vec<SaveStubDto>, String> {
    let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.refresh_catalog()
}

/// Dashboard status: detection flags, counts, hints, loaded stub.
///
/// # Errors
///
/// Lock poison.
#[tauri::command]
pub fn get_dashboard(state: State<'_, AppState>) -> Result<DashboardDto, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    Ok(session.dashboard())
}

/// Pasteable path candidates when auto-detect fails.
#[tauri::command]
pub fn detection_hints() -> Vec<String> {
    crate::session::detection_hints()
}

/// Bind analysis + SQL session by filename stub.
///
/// # Arguments
///
/// * `name` — stub (`autosave` or `autosave.v3`).
/// * `location` — optional `local` / `steam_cloud`.
/// * `solve_opts_json` — optional SolveOpts JSON (default `{}`).
///
/// # Errors
///
/// Ambiguous/missing stub, missing defs/tokens, load/solve failure (message string).
#[tauri::command]
pub fn use_save(
    state: State<'_, AppState>,
    name: String,
    location: Option<String>,
    solve_opts_json: Option<String>,
) -> Result<String, String> {
    let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    let opts = solve_opts_json.unwrap_or_else(|| "{}".into());
    session.use_save(&name, location.as_deref(), &opts)
}

/// Prices JSON for the bound analysis session (`vic3-api`).
///
/// # Errors
///
/// No loaded analysis, or lock poison.
#[tauri::command]
pub fn loaded_prices(state: State<'_, AppState>) -> Result<String, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.loaded_prices_json()
}

/// Alerts JSON for the bound analysis session.
///
/// # Errors
///
/// No loaded analysis, or lock poison.
#[tauri::command]
pub fn loaded_alerts(state: State<'_, AppState>) -> Result<String, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.loaded_alerts_json()
}

/// Gaps JSON for `goal` against the bound session.
///
/// # Arguments
///
/// * `goal` — DSL string (see `docs/dsl.md`).
///
/// # Errors
///
/// No loaded analysis, goal parse/plan failure, or lock poison.
#[tauri::command]
pub fn loaded_gaps(state: State<'_, AppState>, goal: String) -> Result<String, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.loaded_gaps_json(&goal)
}

/// Advanced Query: one read-only statement → `{ columns, rows, row_count }` JSON.
///
/// Same result shape as MCP `query` (`docs/mcp.md` / `docs/sql.md`).
///
/// # Errors
///
/// SQL / session errors as strings; lock poison.
#[tauri::command]
pub fn sql_query(state: State<'_, AppState>, sql: String) -> Result<String, String> {
    let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.sql_query(&sql)
}

/// Advanced Query docs panel: `docs/sql.md` + UDF index (future `vic3://docs/sql`).
///
/// # Errors
///
/// Lock poison.
#[tauri::command]
pub fn sql_docs(state: State<'_, AppState>) -> Result<SqlDocsDto, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    Ok(session.sql_docs())
}

/// Query the current detection and configuration status of supported MCP clients on this OS.
#[tauri::command]
pub fn get_mcp_status() -> Vec<vic3_catalog::McpClientStatus> {
    vic3_catalog::McpClientKind::supported_on_current_os()
        .into_iter()
        .map(|c| c.status())
        .collect()
}

/// Enable or disable MCP configuration for a specific AI client.
#[tauri::command]
pub fn toggle_mcp_client(
    client_id: String,
    enabled: bool,
) -> Result<vic3_catalog::McpClientStatus, String> {
    let client = vic3_catalog::McpClientKind::from_id_or_alias(&client_id)
        .ok_or_else(|| format!("unknown client '{client_id}'"))?;
    if enabled {
        let binary = vic3_catalog::resolve_mcp_binary(None);
        vic3_catalog::install_client_config(client, &binary, false)?;
    } else {
        vic3_catalog::uninstall_client_config(client, false)?;
    }
    Ok(client.status())
}

fn restart_watch(app: &AppHandle, state: &State<'_, AppState>) -> Result<(), String> {
    let roots = {
        let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
        session.save_dirs().to_vec()
    };
    if let Some(slot) = app.try_state::<Mutex<Option<WatchHandle>>>() {
        let mut guard = slot.lock().map_err(|_| "watch lock poisoned")?;
        watch::restart_watcher(app, &mut guard, roots);
    }
    Ok(())
}

/// Register managed state and start the initial save-dir watcher.
///
/// # Errors
///
/// Propagates Tauri setup errors. Config load failure falls back to a temp
/// app-data dir (logged) rather than failing startup.
pub fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let session = CompanionSession::open(None).unwrap_or_else(|err| {
        eprintln!("vic3-analyzer: config load failed ({err}); using empty defaults");
        let fallback = tempfile_app_data();
        CompanionSession::open(Some(fallback)).expect("fallback session")
    });
    let roots = session.save_dirs().to_vec();
    app.manage(AppState::new(session));
    let handle = watch::spawn_save_watcher(app.handle().clone(), roots);
    app.manage(Mutex::new(handle));
    Ok(())
}

fn tempfile_app_data() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vic3-analyzer-fallback-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}
