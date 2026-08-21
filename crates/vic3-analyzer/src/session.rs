//! Companion session: config, catalog, `vic3-sql` engine, and analysis loads.
//!
//! Advanced Query and the Saves tab share one [`SqlEngine`]: `use_save` binds
//! both the process analysis session (for `loaded_*` JSON) and DataFusion
//! `active.*` / TVFs. Catalog-only SQL (`saves`, `latest.*`) works before bind.
//!
//! ```text
//! AppConfig → scan_roots → SaveCatalog
//!                ↓
//!         SqlEngine::use_save
//!                ↓
//!    sql_query  +  loaded_* (vic3-api)
//! ```
//!
//! MCP uses the same catalog/config/SQL crates via `vic3-mcp` (separate process).
//! On-disk config and defs-cache resolution match `vic3-analyzer mcp`
//! ([`vic3_catalog::DesktopConfig`], [`vic3_api::ensure_defs_blob`]); GUI and
//! MCP do not share RAM.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::json;
use vic3_catalog::{
    default_game_dir_candidates, default_local_save_dir, scan_roots, AppConfig, DesktopConfig,
    SaveCatalog, SaveEntry, SaveLocation,
};
use vic3_sql::{batches_to_json, EngineLoadOpts, SqlEngine, UseSaveRequest, UseSaveResult};

use crate::dto::{parse_location, ConfigDto, DashboardDto, SaveStubDto};

/// Body of `docs/sql.md` — single source with future MCP `vic3://docs/sql`.
pub const SQL_DOCS_MD: &str = include_str!("../../../docs/sql.md");

/// Short UDF/TVF index shown beside the full SQL doc in Advanced Query.
pub const SQL_UDF_INDEX_MD: &str = r#"# SQL functions (index)

Registered on the active session after `use_save` (see `docs/sql.md` for columns).

## Scalars
- `good_price(good TEXT)` → FLOAT
- `army_power()` → FLOAT
- `player_tag()` → TEXT
- `is_underemployed(state_id BIGINT)` → BOOLEAN

## Diagnostics TVFs
- `alerts([scope])` — default player-scoped; `'all'` = full save
- `suggest_mitigations([scope])` — heuristic rows; `'player'` / `'all'`
- `shortage_analysis(good TEXT)` — `NULL` = all shortage kinds
- `building_staffing(state_id BIGINT)`

## Planning TVFs
- `plan(goal TEXT [, max_days [, label]])`
- `gaps(goal TEXT)`

Catalog (no session): `saves`. Fact tables: short names / `active.*` are player-scoped; use `world_*` for the full save; `latest.*` at read time.
"#;

/// Shared GUI state (mutex for Tauri managed state).
pub struct AppState {
    /// Inner companion session.
    pub inner: Mutex<CompanionSession>,
}

impl AppState {
    /// Wrap a session for `app.manage`.
    pub fn new(session: CompanionSession) -> Self {
        Self {
            inner: Mutex::new(session),
        }
    }
}

/// In-memory companion session backed by `vic3-catalog` + `vic3-sql` + `vic3-api`.
///
/// Config/defs resolution matches [`vic3_mcp::McpRuntime`] via
/// [`DesktopConfig`] and [`vic3_api::ensure_defs_blob`] — same files on disk,
/// separate RAM session from MCP.
pub struct CompanionSession {
    app_data: PathBuf,
    config_path: PathBuf,
    config: AppConfig,
    catalog: SaveCatalog,
    loaded_stub: Option<String>,
    /// Lazily opened; rebuilt when config/catalog roots or solve opts change.
    sql: Option<SqlEngine>,
    /// SolveOpts JSON last baked into [`EngineLoadOpts`] (drives rebuilds).
    sql_solve_opts: String,
}

impl CompanionSession {
    /// Load config from the platform app-data path (or a test override).
    ///
    /// # Arguments
    ///
    /// * `app_data` — `None` uses [`app_data_dir`]; `Some` for tests.
    ///
    /// # Errors
    ///
    /// App-data / config / initial catalog scan failures as strings.
    pub fn open(app_data: Option<PathBuf>) -> Result<Self, String> {
        let DesktopConfig {
            app_data,
            config_path,
            config,
        } = DesktopConfig::open(app_data).map_err(|e| e.to_string())?;
        let mut session = Self {
            app_data,
            config_path,
            config,
            catalog: SaveCatalog::default(),
            loaded_stub: None,
            sql: None,
            sql_solve_opts: "{}".into(),
        };
        session.refresh_catalog()?;
        Ok(session)
    }

    /// Current Settings DTO (includes absolute `config_path`).
    pub fn config_dto(&self) -> ConfigDto {
        ConfigDto::from_config(&self.config, &self.config_path)
    }

    /// Configured save-root directories (watcher input).
    pub fn save_dirs(&self) -> &[PathBuf] {
        &self.config.save_dirs
    }

    /// Apply Settings DTO, persist, drop SQL engine, refresh catalog.
    ///
    /// # Errors
    ///
    /// Config write or catalog scan failure.
    pub fn apply_config(&mut self, dto: ConfigDto) -> Result<ConfigDto, String> {
        // Preserve path from disk; ignore client-supplied config_path for writes.
        let path = self.config_path.clone();
        self.config = dto.into_config();
        if self.config.auto_detect {
            self.config.apply_auto_detect();
        }
        self.config.save(&path).map_err(|e| e.to_string())?;
        // Defs/tokens/save roots may have changed — drop the engine so the next
        // query/use_save rebuilds with fresh EngineLoadOpts + catalog.
        self.drop_sql();
        self.refresh_catalog()?;
        Ok(self.config_dto())
    }

    /// Clear overrides, auto-detect, persist, refresh.
    ///
    /// # Errors
    ///
    /// Config write or catalog scan failure.
    pub fn reset_to_auto_detect(&mut self) -> Result<ConfigDto, String> {
        self.config.reset_to_auto_detect();
        self.config
            .save(&self.config_path)
            .map_err(|e| e.to_string())?;
        // Auto-detect may rewrite roots/defs — rebuild the engine on next use.
        self.drop_sql();
        self.refresh_catalog()?;
        Ok(self.config_dto())
    }

    /// Rescan roots into the in-memory catalog (and live SQL engine if open).
    ///
    /// # Errors
    ///
    /// Catalog I/O / SQL refresh failure.
    pub fn refresh_catalog(&mut self) -> Result<Vec<SaveStubDto>, String> {
        let roots = self.config.save_roots();
        self.catalog = scan_roots(&roots).map_err(|e| e.to_string())?;
        if let Some(eng) = self.sql.as_ref() {
            eng.refresh_catalog(&roots).map_err(|e| e.to_string())?;
        }
        Ok(self.list_stubs())
    }

    /// Agent/UI stub rows (no absolute paths).
    pub fn list_stubs(&self) -> Vec<SaveStubDto> {
        self.catalog
            .entries()
            .iter()
            .map(SaveStubDto::from)
            .collect()
    }

    /// Rolling dashboard payload for first-launch / Settings status.
    pub fn dashboard(&self) -> DashboardDto {
        let game_detected = self
            .config
            .game_dir
            .as_ref()
            .is_some_and(|p| vic3_catalog::is_valid_game_dir(p));
        DashboardDto {
            config: self.config_dto(),
            game_detected,
            save_root_count: self.config.save_dirs.len(),
            save_count: self.catalog.len(),
            loaded_stub: self.loaded_stub.clone(),
            detection_hints: detection_hints(),
        }
    }

    /// Resolve stub → path, bind SQL + analysis session, return summary JSON.
    ///
    /// Goes through [`SqlEngine::use_save`] so Advanced Query sees the same
    /// binding as `loaded_prices` / MCP.
    ///
    /// # Arguments
    ///
    /// * `name` — filename stub.
    /// * `location` — optional `local` / `steam_cloud`.
    /// * `solve_opts_json` — SolveOpts JSON baked into [`EngineLoadOpts`].
    ///
    /// # Errors
    ///
    /// Ambiguous/missing stub, defs/tokens, load/solve, or SQL engine errors.
    pub fn use_save(
        &mut self,
        name: &str,
        location: Option<&str>,
        solve_opts_json: &str,
    ) -> Result<String, String> {
        let loc = parse_location(location)?;
        let opts = if solve_opts_json.trim().is_empty() {
            "{}"
        } else {
            solve_opts_json
        };
        self.ensure_sql(opts)?;
        let eng = self.sql.as_ref().expect("ensure_sql");
        let result: UseSaveResult = block_on(eng.use_save(UseSaveRequest {
            name: Some(name.to_string()),
            location: loc,
            ..Default::default()
        }))?;
        self.loaded_stub = Some(result.name.clone());
        // Same shape the Saves tab already parses (`summary.tag` / `summary.date`).
        Ok(json!({
            "summary": {
                "tag": result.country,
                "date": result.in_game_date,
            },
            "sql": {
                "name": result.name,
                "kind": result.kind,
                "loaded": result.loaded,
            }
        })
        .to_string())
    }

    /// Run one read-only SQL statement; returns MCP/UI JSON (`columns`/`rows`).
    ///
    /// # Errors
    ///
    /// Engine open / SQL execution / JSON encode failures.
    pub fn sql_query(&mut self, sql: &str) -> Result<String, String> {
        self.ensure_sql(&self.sql_solve_opts.clone())?;
        let eng = self.sql.as_ref().expect("ensure_sql");
        let batches = vic3_sql::query(eng, sql).map_err(|e| e.to_string())?;
        let value = batches_to_json(&batches).map_err(|e| e.to_string())?;
        serde_json::to_string(&value).map_err(|e| e.to_string())
    }

    /// Markdown for the in-app docs panel (`docs/sql.md` + UDF index).
    pub fn sql_docs(&self) -> SqlDocsDto {
        SqlDocsDto {
            sql_md: SQL_DOCS_MD.to_string(),
            udf_md: SQL_UDF_INDEX_MD.to_string(),
        }
    }

    /// Bound-session prices JSON via `vic3-api`.
    ///
    /// # Errors
    ///
    /// [`vic3_api::ApiError`] when no analysis is loaded.
    pub fn loaded_prices_json(&self) -> Result<String, String> {
        vic3_api::loaded_prices_json().map_err(|e| e.to_string())
    }

    /// Bound-session alerts JSON via `vic3-api`.
    ///
    /// # Errors
    ///
    /// [`vic3_api::ApiError`] when no analysis is loaded.
    pub fn loaded_alerts_json(&self) -> Result<String, String> {
        vic3_api::loaded_alerts_json().map_err(|e| e.to_string())
    }

    /// Bound-session gaps JSON for `goal`.
    ///
    /// # Errors
    ///
    /// No analysis, or goal/gaps evaluation failure.
    pub fn loaded_gaps_json(&self, goal: &str) -> Result<String, String> {
        vic3_api::loaded_gaps_json(goal).map_err(|e| e.to_string())
    }

    /// Resolve a stub against the in-memory catalog.
    ///
    /// # Errors
    ///
    /// [`vic3_catalog::CatalogError`] as a string (not found / ambiguous).
    pub fn resolve_entry(
        &self,
        name: &str,
        location: Option<SaveLocation>,
    ) -> Result<&SaveEntry, String> {
        self.catalog
            .resolve(name, location, None)
            .map_err(|e| e.to_string())
    }

    fn drop_sql(&mut self) {
        self.sql = None;
    }

    /// Open or rebuild the DataFusion engine against the current catalog.
    ///
    /// Rebuilds when `solve_opts_json` differs from the opts baked into the
    /// current engine (HostState owns `EngineLoadOpts` immutably).
    fn ensure_sql(&mut self, solve_opts_json: &str) -> Result<(), String> {
        if self.sql.is_some() && self.sql_solve_opts == solve_opts_json {
            return Ok(());
        }
        let load = self.engine_load_opts(solve_opts_json)?;
        let catalog = self.catalog.clone();
        let eng = block_on(SqlEngine::with_catalog(catalog, load))?;
        self.sql = Some(eng);
        self.sql_solve_opts = solve_opts_json.to_string();
        Ok(())
    }

    fn engine_load_opts(&self, solve_opts_json: &str) -> Result<EngineLoadOpts, String> {
        let defs = self.defs_blob_for_engine()?;
        let mut load = EngineLoadOpts::new(defs);
        if let Some(tokens) = &self.config.tokens_path {
            load = load.with_tokens(tokens.clone());
        }
        let opts = if solve_opts_json.trim().is_empty() {
            "{}"
        } else {
            solve_opts_json
        };
        load.solve_opts_json = opts.to_string();
        Ok(load)
    }

    /// Prefer a real defs blob; if missing, use a placeholder path so catalog
    /// SQL (`saves`) still works — `use_save` then fails with a clear API error.
    fn defs_blob_for_engine(&self) -> Result<PathBuf, String> {
        match self.try_ensure_defs_blob() {
            Ok(path) => Ok(path),
            Err(_) => Ok(self.app_data.join(vic3_api::DEFS_CACHE_NAME)),
        }
    }

    /// Same path order as MCP bootstrap: explicit blob → app-data cache →
    /// build from game_dir ([`vic3_api::ensure_defs_blob`]).
    fn try_ensure_defs_blob(&self) -> Result<PathBuf, String> {
        if let Some(game) = &self.config.game_dir {
            if !vic3_catalog::is_valid_game_dir(game) {
                return Err(format!(
                    "game_dir is not a valid Victoria 3 game tree: {}",
                    game.display()
                ));
            }
        }
        vic3_api::ensure_defs_blob(
            self.config.defs_blob.as_deref(),
            self.config.game_dir.as_deref(),
            &self.app_data,
        )
        .map_err(|e| e.to_string())
    }
}

/// Docs panel payload for Advanced Query / future `vic3://docs/sql`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SqlDocsDto {
    /// Full body of `docs/sql.md` (embedded via [`SQL_DOCS_MD`]).
    pub sql_md: String,
    /// Short UDF/TVF index ([`SQL_UDF_INDEX_MD`]) shown above the full doc.
    pub udf_md: String,
}

fn block_on<F, T>(fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, vic3_sql::SqlError>>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;
    rt.block_on(fut).map_err(|e| e.to_string())
}

/// Pasteable path hints when auto-detect fails (Settings modal copy).
pub fn detection_hints() -> Vec<String> {
    let mut hints = Vec::new();
    hints.push("Game folder should contain a `common/` directory (…/Victoria 3/game).".into());
    for candidate in default_game_dir_candidates() {
        hints.push(format!("Game candidate: {}", candidate.display()));
    }
    if let Some(local) = default_local_save_dir() {
        hints.push(format!("Local saves: {}", local.display()));
    }
    hints.push("macOS Finder: Go → Go to Folder… (⇧⌘G) and paste a path above.".into());
    hints.push("Windows: paste into Explorer address bar; Linux: use the path as-is.".into());
    hints
}

/// Test helper: open a session rooted at `app_data` without touching the real home dir.
#[cfg(test)]
pub fn open_for_test(app_data: &std::path::Path) -> CompanionSession {
    CompanionSession::open(Some(app_data.to_path_buf())).expect("open test session")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write_save(dir: &std::path::Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), b"SAV").unwrap();
    }

    fn fixture_save() -> Vec<u8> {
        fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../vic3-load/tests/fixtures/plaintext.txt"),
        )
        .expect("plaintext fixture")
    }

    fn fixture_defs_blob() -> Vec<u8> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures");
        let defs = vic3_defs::load_from_path(&root).expect("defs fixture");
        vic3_defs::encode_blob(&defs).expect("encode")
    }

    #[test]
    fn config_round_trip_and_list_stubs() {
        let tmp = tempfile::tempdir().unwrap();
        let saves = tmp.path().join("saves");
        write_save(&saves, "autosave.v3");
        write_save(&saves, "Campaign.v3");

        let mut session = open_for_test(tmp.path());
        let dto = ConfigDto {
            game_dir: None,
            defs_blob: None,
            save_dirs: vec![saves.display().to_string()],
            tokens_path: None,
            auto_detect: false,
            config_path: session.config_path.display().to_string(),
        };
        session.apply_config(dto).unwrap();
        assert!(session.config_path.exists());

        let stubs = session.list_stubs();
        assert_eq!(stubs.len(), 2);
        assert!(stubs.iter().any(|s| s.name == "autosave"));
        assert!(stubs.iter().any(|s| s.name == "Campaign"));

        let reloaded = CompanionSession::open(Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(reloaded.config.save_dirs, vec![saves]);
        assert_eq!(reloaded.list_stubs().len(), 2);
    }

    #[test]
    fn reset_clears_overrides_and_rescans() {
        let tmp = tempfile::tempdir().unwrap();
        let saves = tmp.path().join("saves");
        write_save(&saves, "named.v3");
        let mut session = open_for_test(tmp.path());
        session
            .apply_config(ConfigDto {
                game_dir: Some(tmp.path().join("not-a-game").display().to_string()),
                defs_blob: None,
                save_dirs: vec![saves.display().to_string()],
                tokens_path: None,
                auto_detect: false,
                config_path: String::new(),
            })
            .unwrap();
        assert_eq!(session.list_stubs().len(), 1);

        // Reset enables auto_detect; without real Steam paths save_dirs may empty or fill.
        let dto = session.reset_to_auto_detect().unwrap();
        assert!(dto.auto_detect);
    }

    #[test]
    fn use_save_binds_sql_and_analysis() {
        vic3_api::clear_analysis();
        let tmp = tempfile::tempdir().unwrap();
        let saves = tmp.path().join("saves");
        fs::create_dir_all(&saves).unwrap();
        fs::write(saves.join("autosave.v3"), fixture_save()).unwrap();
        let blob_path = tmp.path().join("defs.postcard");
        fs::write(&blob_path, fixture_defs_blob()).unwrap();

        let mut session = open_for_test(tmp.path());
        session
            .apply_config(ConfigDto {
                game_dir: None,
                defs_blob: Some(blob_path.display().to_string()),
                save_dirs: vec![saves.display().to_string()],
                tokens_path: None,
                auto_detect: false,
                config_path: String::new(),
            })
            .unwrap();

        let json = session.use_save("autosave", None, "{}").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["summary"]["tag"], "GER");
        assert_eq!(session.loaded_stub.as_deref(), Some("autosave"));

        let prices = session.loaded_prices_json().unwrap();
        assert!(prices.contains("\"residual\""));
        let alerts = session.loaded_alerts_json().unwrap();
        assert!(alerts.contains("\"alerts\""));

        let query = session
            .sql_query("SELECT tag FROM countries WHERE tag = 'GER'")
            .unwrap();
        let q: serde_json::Value = serde_json::from_str(&query).unwrap();
        assert_eq!(q["row_count"], 1);
        assert_eq!(q["columns"][0], "tag");
        assert_eq!(q["rows"][0][0], "GER");

        let saves_q = session
            .sql_query("SELECT name, loaded FROM saves WHERE name = 'autosave'")
            .unwrap();
        let s: serde_json::Value = serde_json::from_str(&saves_q).unwrap();
        assert_eq!(s["rows"][0][1], true);

        vic3_api::clear_analysis();
    }

    #[test]
    fn sql_docs_include_contract() {
        let tmp = tempfile::tempdir().unwrap();
        let session = open_for_test(tmp.path());
        let docs = session.sql_docs();
        assert!(docs.sql_md.contains("Advanced Query UI"));
        assert!(docs.udf_md.contains("alerts()"));
        assert!(SQL_DOCS_MD.len() > 1000);
    }

    #[test]
    fn catalog_sql_without_use_save() {
        let tmp = tempfile::tempdir().unwrap();
        let saves = tmp.path().join("saves");
        write_save(&saves, "autosave.v3");
        let mut session = open_for_test(tmp.path());
        session
            .apply_config(ConfigDto {
                game_dir: None,
                defs_blob: None,
                save_dirs: vec![saves.display().to_string()],
                tokens_path: None,
                auto_detect: false,
                config_path: String::new(),
            })
            .unwrap();
        let json = session
            .sql_query("SELECT name FROM saves ORDER BY name")
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["rows"][0][0], "autosave");
    }

    #[test]
    fn dashboard_exposes_hints() {
        let tmp = tempfile::tempdir().unwrap();
        let session = open_for_test(tmp.path());
        let dash = session.dashboard();
        assert!(!dash.detection_hints.is_empty());
        assert!(dash
            .detection_hints
            .iter()
            .any(|h| h.contains("common/") || h.contains("Game")));
    }
}
