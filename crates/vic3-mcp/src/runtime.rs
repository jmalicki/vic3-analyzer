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

use serde_json::{json, Value};
use tokio::sync::Mutex;
use vic3_catalog::{is_valid_game_dir, scan_roots, AppConfig, DesktopConfig, SaveEntry, SaveRoot};
use vic3_prices::{preview, ExtraLevelsDelta, GoodPrice, PricesResult, SolveOpts, WorldDelta};
use vic3_sql::{
    ActiveSessionInfo, EngineLoadOpts, SessionBinding, SqlEngine, SqlError, UseSaveRequest,
    UseSaveResult,
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

    /// Active analysis binding after `use_save` / `bind`, if any.
    pub async fn active_binding(&self) -> Option<std::sync::Arc<SessionBinding>> {
        let eng = self.engine.lock().await;
        eng.active_binding()
    }

    /// Preview a [`WorldDelta`] on the bound session (warm-started); compact JSON.
    ///
    /// Does not mutate the session world or prices. Unbound → error string.
    pub async fn preview_delta(&self, delta: &WorldDelta) -> Result<Value, String> {
        let binding = self
            .active_binding()
            .await
            .ok_or_else(|| "no active save; call use_save first".to_string())?;
        Ok(compact_preview(&binding, delta))
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

fn shortage(g: &GoodPrice) -> f64 {
    (g.buy - g.sell).max(0.0)
}

/// Compact before/after goods preview (not a full [`PricesResult`]).
fn compact_preview(binding: &SessionBinding, delta: &WorldDelta) -> Value {
    let baseline: &PricesResult = binding.prices.as_ref();
    let mut opts = SolveOpts::default();
    if !baseline.relative.is_empty() {
        opts.warm_rel = Some(baseline.relative.clone());
    }
    let after = preview(binding.world.as_ref(), binding.defs.as_ref(), delta, opts);

    let before_by_id: std::collections::BTreeMap<&str, &GoodPrice> =
        baseline.goods.iter().map(|g| (g.id.as_str(), g)).collect();

    let mut goods = Vec::new();
    for g_after in &after.goods {
        let (price_before, shortage_before) = match before_by_id.get(g_after.id.as_str()) {
            Some(g) => (g.price, shortage(g)),
            None => (g_after.base, 0.0),
        };
        let price_after = g_after.price;
        let shortage_after = shortage(g_after);
        let d_price = price_after - price_before;
        let d_shortage = shortage_after - shortage_before;
        if d_price.abs() < 1e-9 && d_shortage.abs() < 1e-9 {
            continue;
        }
        goods.push(json!({
            "id": g_after.id,
            "price_before": price_before,
            "price_after": price_after,
            "d_price": d_price,
            "shortage_before": shortage_before,
            "shortage_after": shortage_after,
            "d_shortage": d_shortage,
        }));
    }

    json!({
        "status": after.status.to_string(),
        "residual": after.residual,
        "applied": delta,
        "goods": goods,
        "limitations": after.limitations,
    })
}

/// Resolve sugar args into a [`WorldDelta`], or an error message.
pub fn world_delta_from_sugar(
    binding: &SessionBinding,
    building: Option<&str>,
    extra_levels: Option<u32>,
    building_id: Option<u32>,
    state_id: Option<u32>,
) -> Result<WorldDelta, String> {
    let extra_levels = extra_levels.ok_or_else(|| {
        "sugar preview requires extra_levels (and building or building_id)".to_string()
    })?;

    if let Some(id) = building_id {
        if let Some(want) = state_id {
            let ok = binding
                .world
                .buildings
                .iter()
                .any(|b| b.id == id && b.state == Some(want));
            if !ok {
                return Err(format!("building_id {id} not found in state_id {want}"));
            }
        }
        return Ok(WorldDelta {
            extra_levels: vec![ExtraLevelsDelta {
                building_type_id: None,
                building_id: Some(id),
                extra_levels,
            }],
            ..WorldDelta::default()
        });
    }

    let building = building.ok_or_else(|| {
        "sugar preview requires building or building_id (with extra_levels)".to_string()
    })?;

    if let Some(want_state) = state_id {
        let ids: Vec<u32> = binding
            .world
            .buildings
            .iter()
            .filter(|b| b.type_id == building && b.state == Some(want_state))
            .map(|b| b.id)
            .collect();
        if ids.is_empty() {
            return Err(format!("no building {building:?} in state_id {want_state}"));
        }
        return Ok(WorldDelta {
            extra_levels: ids
                .into_iter()
                .map(|id| ExtraLevelsDelta {
                    building_type_id: None,
                    building_id: Some(id),
                    extra_levels,
                })
                .collect(),
            ..WorldDelta::default()
        });
    }

    Ok(WorldDelta {
        extra_levels: vec![ExtraLevelsDelta {
            building_type_id: Some(building.to_string()),
            building_id: None,
            extra_levels,
        }],
        ..WorldDelta::default()
    })
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
