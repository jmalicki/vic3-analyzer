//! Shared desktop config + SQL engine bootstrap for the MCP process.
//!
//! Mirrors `vic3_analyzer_lib::session::CompanionSession` path resolution so
//! GUI Settings and `vic3-analyzer mcp` read the same allowlists — without
//! depending on the Tauri crate.
//!
//! Flow: [`AppConfig`] → [`scan_roots`] → [`SqlEngine::with_catalog`] → tools.
//! Why this crate (not `vic3-analyzer`): MCP must not depend on Tauri, yet must
//! resolve the same app-data config and defs cache as the companion UI. That is
//! the fat-binary “share crates, separate process” contract in `docs/mcp.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;
use vic3_catalog::{is_valid_game_dir, scan_roots, AppConfig, DesktopConfig, SaveEntry, SaveRoot};
use vic3_sql::{
    ActiveSessionInfo, EngineLoadOpts, SqlEngine, SqlError, UseSaveRequest, UseSaveResult,
};

use crate::brief::campaign_brief_json;

/// Why MCP failed to open (config / catalog / engine).
#[derive(Debug, thiserror::Error)]
pub enum McpBootstrapError {
    /// Human-readable bootstrap failure (paths, defs, I/O).
    #[error("{0}")]
    Message(String),
    /// Propagated from [`SqlEngine`] construction / catalog wiring.
    #[error(transparent)]
    Sql(#[from] SqlError),
}

impl From<String> for McpBootstrapError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

/// Process-local MCP session: config, save roots, and a locked [`SqlEngine`].
///
/// The engine mutex serializes `use_save` rebinding against concurrent `query`
/// calls (DataFusion registration is not assumed concurrent-safe here). GUI and
/// MCP do **not** share RAM in v1 — only the on-disk [`AppConfig`].
pub struct McpRuntime {
    app_data: PathBuf,
    config_path: PathBuf,
    config: AppConfig,
    /// Last known defs status for `vic3://session` (path may be missing).
    defs_status: DefsStatus,
    engine: Arc<Mutex<SqlEngine>>,
}

#[derive(Debug, Clone)]
pub(crate) struct DefsStatus {
    pub ready: bool,
    pub path: Option<PathBuf>,
    pub detail: String,
}

impl McpRuntime {
    /// Load config from platform app-data (or `app_data` override for tests).
    ///
    /// # Arguments
    ///
    /// * `app_data` — `None` uses [`app_data_dir`]; `Some` for tests / fixtures.
    ///
    /// # Errors
    ///
    /// [`McpBootstrapError`] when app-data cannot be created, config cannot be
    /// loaded, catalog scan fails, or the SQL engine cannot open.
    pub async fn open(app_data: Option<PathBuf>) -> Result<Self, McpBootstrapError> {
        let DesktopConfig {
            app_data,
            config_path,
            config,
        } = DesktopConfig::open(app_data).map_err(|e| e.to_string())?;
        Self::from_config(app_data, config_path, config).await
    }

    /// Build a runtime from an already-loaded [`AppConfig`].
    ///
    /// # Errors
    ///
    /// Catalog scan / [`SqlEngine`] construction failures.
    pub async fn from_config(
        app_data: PathBuf,
        config_path: PathBuf,
        config: AppConfig,
    ) -> Result<Self, McpBootstrapError> {
        let roots = config.save_roots();
        let catalog = scan_roots(&roots).map_err(|e| e.to_string())?;
        let (defs_status, load) = resolve_load_opts(&config, &app_data);
        let engine = SqlEngine::with_catalog(catalog, load).await?;
        Ok(Self {
            app_data,
            config_path,
            config,
            defs_status,
            engine: Arc::new(Mutex::new(engine)),
        })
    }

    /// Path of the shared TOML config (same file the GUI Settings writes).
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Platform app-data dir used for config + defs cache.
    pub fn app_data(&self) -> &Path {
        &self.app_data
    }

    /// Number of configured save *directories* (not catalog entry count).
    pub fn save_dir_count(&self) -> usize {
        self.config.save_dirs.len()
    }

    /// Deprecated alias for [`Self::save_dir_count`] (name was easy to confuse
    /// with catalog size).
    pub fn save_count(&self) -> usize {
        self.save_dir_count()
    }

    /// Allowlisted save roots derived from config (local / steam_cloud).
    pub fn save_roots(&self) -> Vec<SaveRoot> {
        self.config.save_roots()
    }

    pub(crate) fn defs_status(&self) -> &DefsStatus {
        &self.defs_status
    }

    /// Shared SQL engine; callers must hold the mutex across await points carefully.
    pub fn engine(&self) -> Arc<Mutex<SqlEngine>> {
        Arc::clone(&self.engine)
    }

    /// Snapshot of catalog rows (agent-facing fields + internal paths).
    ///
    /// # Errors
    ///
    /// [`SqlError`] from the engine catalog view.
    pub async fn catalog_entries(&self) -> Result<Vec<SaveEntry>, SqlError> {
        let eng = self.engine.lock().await;
        eng.catalog_entries()
    }

    /// Active save summary for `vic3://session`, if bound.
    pub async fn active_session(&self) -> Option<ActiveSessionInfo> {
        let eng = self.engine.lock().await;
        eng.active_session()
    }

    /// Rescan allowlisted roots into the engine `saves` table.
    ///
    /// # Errors
    ///
    /// Propagates [`SqlError`] from [`SqlEngine::refresh_catalog`].
    pub async fn refresh_catalog(&self) -> Result<usize, SqlError> {
        let roots = self.save_roots();
        let eng = self.engine.lock().await;
        eng.refresh_catalog(&roots)
    }

    /// Bind / load / solve via host API (not SQL).
    ///
    /// # Errors
    ///
    /// [`SqlError`] — not found, ambiguous stub, missing defs/tokens, load failure.
    pub async fn use_save(&self, req: UseSaveRequest) -> Result<UseSaveResult, SqlError> {
        let eng = self.engine.lock().await;
        eng.use_save(req).await
    }

    /// Run one read-only SQL statement (`docs/sql.md`).
    ///
    /// # Arguments
    ///
    /// * `sql` — single `SELECT` / `WITH` / `EXPLAIN` statement.
    ///
    /// # Errors
    ///
    /// [`SqlError`] — syntax, DDL rejected, no active save, plan timeout, etc.
    pub async fn query(
        &self,
        sql: &str,
    ) -> Result<Vec<datafusion::arrow::array::RecordBatch>, SqlError> {
        let eng = self.engine.lock().await;
        eng.query(sql).await
    }

    /// Compact campaign summary after [`Self::use_save`] (tool `campaign_brief`).
    ///
    /// Reads [`SqlEngine::active_binding`] directly: domestic top goods /
    /// hotspots and a player-scoped alert-kind histogram (same filter as
    /// zero-arg `alerts()`).
    ///
    /// # Errors
    ///
    /// [`SqlError::Unbound`] when no save is bound.
    pub async fn campaign_brief(&self) -> Result<Value, SqlError> {
        let eng = self.engine.lock().await;
        let binding = eng.active_binding().ok_or(SqlError::Unbound)?;
        let session = eng.active_session().ok_or(SqlError::Unbound)?;
        Ok(campaign_brief_json(&session, binding.as_ref()))
    }
}

fn resolve_load_opts(config: &AppConfig, app_data: &Path) -> (DefsStatus, EngineLoadOpts) {
    match ensure_defs_blob(config, app_data) {
        Ok(path) => {
            let mut load = EngineLoadOpts::new(path.clone());
            if let Some(tokens) = &config.tokens_path {
                load = load.with_tokens(tokens.clone());
            }
            (
                DefsStatus {
                    ready: true,
                    path: Some(path),
                    detail: "defs blob ready".into(),
                },
                load,
            )
        }
        Err(detail) => {
            let placeholder = app_data.join(vic3_api::DEFS_CACHE_NAME);
            let mut load = EngineLoadOpts::new(placeholder.clone());
            if let Some(tokens) = &config.tokens_path {
                load = load.with_tokens(tokens.clone());
            }
            (
                DefsStatus {
                    ready: false,
                    path: Some(placeholder),
                    detail,
                },
                load,
            )
        }
    }
}

/// Resolve postcard defs via the shared [`vic3_api::ensure_defs_blob`] helper.
fn ensure_defs_blob(config: &AppConfig, app_data: &Path) -> Result<PathBuf, String> {
    if let Some(game) = &config.game_dir {
        if !is_valid_game_dir(game) {
            return Err(format!(
                "game_dir is not a valid Victoria 3 game tree: {}",
                game.display()
            ));
        }
    }
    vic3_api::ensure_defs_blob(
        config.defs_blob.as_deref(),
        config.game_dir.as_deref(),
        app_data,
    )
    .map_err(|e| e.to_string())
}
