//! Shared desktop config + SQL engine bootstrap for the MCP process.
//!
//! Mirrors [`vic3_analyzer_lib::session::CompanionSession`] path resolution so
//! GUI Settings and `vic3-analyzer mcp` read the same allowlists — without
//! depending on the Tauri crate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;
use vic3_catalog::{
    app_data_dir, is_valid_game_dir, resolve_config_path, scan_roots, AppConfig, SaveEntry,
    SaveRoot,
};
use vic3_sql::{
    ActiveSessionInfo, EngineLoadOpts, SqlEngine, SqlError, UseSaveRequest, UseSaveResult,
};

const DEFS_CACHE_NAME: &str = "defs.postcard";

/// Why MCP failed to open (config / catalog / engine).
#[derive(Debug, thiserror::Error)]
pub enum McpBootstrapError {
    #[error("{0}")]
    Message(String),
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
/// calls (DataFusion registration is not assumed concurrent-safe here).
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
    pub async fn open(app_data: Option<PathBuf>) -> Result<Self, McpBootstrapError> {
        let app_data = match app_data {
            Some(p) => p,
            None => app_data_dir().map_err(|e| e.to_string())?,
        };
        std::fs::create_dir_all(&app_data).map_err(|e| e.to_string())?;
        let config_path = resolve_config_path(&app_data);
        let config = AppConfig::load(&config_path).map_err(|e| e.to_string())?;
        Self::from_config(app_data, config_path, config).await
    }

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

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn app_data(&self) -> &Path {
        &self.app_data
    }

    pub fn save_count(&self) -> usize {
        // Cheap: roots length is not catalog size; callers use tools for exact.
        self.config.save_dirs.len()
    }

    pub fn save_roots(&self) -> Vec<SaveRoot> {
        self.config.save_roots()
    }

    pub(crate) fn defs_status(&self) -> &DefsStatus {
        &self.defs_status
    }

    pub fn engine(&self) -> Arc<Mutex<SqlEngine>> {
        Arc::clone(&self.engine)
    }

    pub async fn catalog_entries(&self) -> Result<Vec<SaveEntry>, SqlError> {
        let eng = self.engine.lock().await;
        eng.catalog_entries()
    }

    pub async fn active_session(&self) -> Option<ActiveSessionInfo> {
        let eng = self.engine.lock().await;
        eng.active_session()
    }

    pub async fn refresh_catalog(&self) -> Result<usize, SqlError> {
        let roots = self.save_roots();
        let eng = self.engine.lock().await;
        eng.refresh_catalog(&roots)
    }

    pub async fn use_save(&self, req: UseSaveRequest) -> Result<UseSaveResult, SqlError> {
        let eng = self.engine.lock().await;
        eng.use_save(req).await
    }

    pub async fn query(
        &self,
        sql: &str,
    ) -> Result<Vec<datafusion::arrow::array::RecordBatch>, SqlError> {
        let eng = self.engine.lock().await;
        eng.query(sql).await
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
            let placeholder = app_data.join(DEFS_CACHE_NAME);
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

/// Resolve postcard defs: explicit blob, or build/cache from `game_dir`.
fn ensure_defs_blob(config: &AppConfig, app_data: &Path) -> Result<PathBuf, String> {
    if let Some(blob) = &config.defs_blob {
        if blob.is_file() {
            return Ok(blob.clone());
        }
        return Err(format!("defs_blob not found: {}", blob.display()));
    }
    let cache = app_data.join(DEFS_CACHE_NAME);
    if cache.is_file() {
        return Ok(cache);
    }
    let game = config.game_dir.as_ref().ok_or_else(|| {
        "no game_dir or defs_blob configured — set paths in Settings or enable auto-detect"
            .to_string()
    })?;
    if !is_valid_game_dir(game) {
        return Err(format!(
            "game_dir is not a valid Victoria 3 game tree: {}",
            game.display()
        ));
    }
    let bytes = vic3_api::defs_blob_from_game(game).map_err(|e| e.to_string())?;
    std::fs::write(&cache, &bytes).map_err(|e| e.to_string())?;
    Ok(cache)
}
