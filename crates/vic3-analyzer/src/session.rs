//! Companion session: config, catalog, and path-based analysis loads.

use std::path::PathBuf;
use std::sync::Mutex;

use vic3_catalog::{
    app_data_dir, default_game_dir_candidates, default_local_save_dir, resolve_config_path,
    scan_roots, AppConfig, SaveCatalog, SaveEntry, SaveLocation,
};

use crate::dto::{parse_location, ConfigDto, DashboardDto, SaveStubDto};

const DEFS_CACHE_NAME: &str = "defs.postcard";

/// Shared GUI state (mutex for Tauri managed state).
pub struct AppState {
    pub inner: Mutex<CompanionSession>,
}

impl AppState {
    pub fn new(session: CompanionSession) -> Self {
        Self {
            inner: Mutex::new(session),
        }
    }
}

/// In-memory companion session backed by `vic3-catalog` + `vic3-api`.
#[derive(Debug)]
pub struct CompanionSession {
    app_data: PathBuf,
    config_path: PathBuf,
    config: AppConfig,
    catalog: SaveCatalog,
    loaded_stub: Option<String>,
}

impl CompanionSession {
    /// Load config from the platform app-data path (or a test override).
    pub fn open(app_data: Option<PathBuf>) -> Result<Self, String> {
        let app_data = match app_data {
            Some(p) => p,
            None => app_data_dir().map_err(|e| e.to_string())?,
        };
        std::fs::create_dir_all(&app_data).map_err(|e| e.to_string())?;
        let config_path = resolve_config_path(&app_data);
        let config = AppConfig::load(&config_path).map_err(|e| e.to_string())?;
        let mut session = Self {
            app_data,
            config_path,
            config,
            catalog: SaveCatalog::default(),
            loaded_stub: None,
        };
        session.refresh_catalog()?;
        Ok(session)
    }

    pub fn config_dto(&self) -> ConfigDto {
        ConfigDto::from_config(&self.config, &self.config_path)
    }

    pub fn save_dirs(&self) -> &[PathBuf] {
        &self.config.save_dirs
    }

    pub fn apply_config(&mut self, dto: ConfigDto) -> Result<ConfigDto, String> {
        // Preserve path from disk; ignore client-supplied config_path for writes.
        let path = self.config_path.clone();
        self.config = dto.into_config();
        if self.config.auto_detect {
            self.config.apply_auto_detect();
        }
        self.config.save(&path).map_err(|e| e.to_string())?;
        self.refresh_catalog()?;
        Ok(self.config_dto())
    }

    pub fn reset_to_auto_detect(&mut self) -> Result<ConfigDto, String> {
        self.config.reset_to_auto_detect();
        self.config
            .save(&self.config_path)
            .map_err(|e| e.to_string())?;
        self.refresh_catalog()?;
        Ok(self.config_dto())
    }

    pub fn refresh_catalog(&mut self) -> Result<Vec<SaveStubDto>, String> {
        let roots = self.config.save_roots();
        self.catalog = scan_roots(&roots).map_err(|e| e.to_string())?;
        Ok(self.list_stubs())
    }

    pub fn list_stubs(&self) -> Vec<SaveStubDto> {
        self.catalog
            .entries()
            .iter()
            .map(SaveStubDto::from)
            .collect()
    }

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

    /// Resolve stub → path, load defs, call [`vic3_api::load_analysis_from_paths`].
    pub fn use_save(
        &mut self,
        name: &str,
        location: Option<&str>,
        solve_opts_json: &str,
    ) -> Result<String, String> {
        let loc = parse_location(location)?;
        let entry = self
            .catalog
            .resolve(name, loc, None)
            .map_err(|e| e.to_string())?;
        let save_path = entry
            .path
            .as_ref()
            .ok_or_else(|| "catalog entry missing path".to_string())?
            .clone();
        let stub_name = entry.name.clone();
        let tokens = self.config.tokens_path.as_deref();
        let defs_path = self.ensure_defs_blob()?;
        let opts = if solve_opts_json.trim().is_empty() {
            "{}"
        } else {
            solve_opts_json
        };
        let json = vic3_api::load_analysis_from_paths(&save_path, tokens, &defs_path, opts)
            .map_err(|e| e.to_string())?;
        self.loaded_stub = Some(stub_name);
        Ok(json)
    }

    pub fn loaded_prices_json(&self) -> Result<String, String> {
        vic3_api::loaded_prices_json().map_err(|e| e.to_string())
    }

    pub fn loaded_alerts_json(&self) -> Result<String, String> {
        vic3_api::loaded_alerts_json().map_err(|e| e.to_string())
    }

    pub fn loaded_gaps_json(&self, goal: &str) -> Result<String, String> {
        vic3_api::loaded_gaps_json(goal).map_err(|e| e.to_string())
    }

    pub fn resolve_entry(
        &self,
        name: &str,
        location: Option<SaveLocation>,
    ) -> Result<&SaveEntry, String> {
        self.catalog
            .resolve(name, location, None)
            .map_err(|e| e.to_string())
    }

    fn ensure_defs_blob(&self) -> Result<PathBuf, String> {
        if let Some(blob) = &self.config.defs_blob {
            if blob.is_file() {
                return Ok(blob.clone());
            }
            return Err(format!("defs_blob not found: {}", blob.display()));
        }
        let cache = self.app_data.join(DEFS_CACHE_NAME);
        let game = self.config.game_dir.as_ref().ok_or_else(|| {
            "no game_dir or defs_blob configured — set paths in Settings or enable auto-detect"
                .to_string()
        })?;
        if !vic3_catalog::is_valid_game_dir(game) {
            return Err(format!(
                "game_dir is not a valid Victoria 3 game tree: {}",
                game.display()
            ));
        }
        let bytes = vic3_api::defs_blob_from_game(game).map_err(|e| e.to_string())?;
        std::fs::write(&cache, &bytes).map_err(|e| e.to_string())?;
        Ok(cache)
    }
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
    fn use_save_loads_analysis_via_api() {
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
        vic3_api::clear_analysis();
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
