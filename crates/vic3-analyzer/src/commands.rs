//! Tauri invoke handlers for the companion UI.

use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::dto::{ConfigDto, DashboardDto, SaveStubDto};
use crate::session::{AppState, CompanionSession};
use crate::watch::{self, WatchHandle};

/// Placeholder invoke proving `vic3-api` is linked into the desktop binary.
#[tauri::command]
pub fn api_ping() -> Result<&'static str, String> {
    let _ = vic3_api::ApiError::NoLoadedAnalysis;
    Ok("pong")
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<ConfigDto, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    Ok(session.config_dto())
}

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

#[tauri::command]
pub fn reset_config(app: AppHandle, state: State<'_, AppState>) -> Result<ConfigDto, String> {
    let dto = {
        let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
        session.reset_to_auto_detect()?
    };
    restart_watch(&app, &state)?;
    Ok(dto)
}

#[tauri::command]
pub fn list_saves(state: State<'_, AppState>) -> Result<Vec<SaveStubDto>, String> {
    let mut session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.refresh_catalog()
}

#[tauri::command]
pub fn get_dashboard(state: State<'_, AppState>) -> Result<DashboardDto, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    Ok(session.dashboard())
}

#[tauri::command]
pub fn detection_hints() -> Vec<String> {
    crate::session::detection_hints()
}

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

#[tauri::command]
pub fn loaded_prices(state: State<'_, AppState>) -> Result<String, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.loaded_prices_json()
}

#[tauri::command]
pub fn loaded_alerts(state: State<'_, AppState>) -> Result<String, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.loaded_alerts_json()
}

#[tauri::command]
pub fn loaded_gaps(state: State<'_, AppState>, goal: String) -> Result<String, String> {
    let session = state.inner.lock().map_err(|_| "state lock poisoned")?;
    session.loaded_gaps_json(&goal)
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
