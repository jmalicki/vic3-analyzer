//! Shared desktop / MCP app config (TOML or JSON under app data).
//!
//! GUI Settings and `vic3-analyzer mcp` read/write the same file. Keys match
//! `docs/desktop.md`. Auto-detect fills missing `game_dir` / empty `save_dirs`
//! from allowlisted Steam / Paradox layouts only — never a whole-home scan.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CatalogError;
use crate::paths::{
    detect_game_dir, detect_save_roots, is_valid_game_dir, roots_from_paths, SaveRoot,
    CONFIG_FILE_JSON, CONFIG_FILE_TOML,
};

/// Persisted knobs shared by Tauri Settings and `vic3-analyzer mcp`.
///
/// Default format is TOML (`config.toml`); JSON is accepted when the path ends
/// in `.json`. Missing file + `auto_detect` → discovery defaults without writing
/// until [`AppConfig::save`] / Settings persist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Absolute path to `…/Victoria 3/game`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_dir: Option<PathBuf>,
    /// Optional prebuilt postcard defs blob (skips live install read when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defs_blob: Option<PathBuf>,
    /// Absolute save-root directories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub save_dirs: Vec<PathBuf>,
    /// Optional binary/ironman token map path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_path: Option<PathBuf>,
    /// When true, fill missing/invalid paths from auto-detect on load.
    #[serde(default = "default_true")]
    pub auto_detect: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            game_dir: None,
            defs_blob: None,
            save_dirs: Vec::new(),
            tokens_path: None,
            auto_detect: true,
        }
    }
}

impl AppConfig {
    /// Load from `path`, applying auto-detect when enabled.
    ///
    /// # Arguments
    ///
    /// * `path` — absolute path ending in `.toml` or `.json`. Missing file is
    ///   treated as defaults (not an error).
    ///
    /// # Errors
    ///
    /// * [`CatalogError::Io`] — read failure.
    /// * [`CatalogError::InvalidConfig`] — parse failure.
    /// * [`CatalogError::UnsupportedConfigFormat`] — other extension.
    pub fn load(path: &Path) -> Result<Self, CatalogError> {
        if !path.exists() {
            let mut cfg = Self::default();
            if cfg.auto_detect {
                cfg.apply_auto_detect();
            }
            return Ok(cfg);
        }
        let bytes = fs::read(path).map_err(|e| CatalogError::io(path, e))?;
        let mut cfg = parse_config_bytes(path, &bytes)?;
        if cfg.auto_detect {
            cfg.apply_auto_detect();
        }
        Ok(cfg)
    }

    /// Write TOML or JSON based on the destination extension.
    ///
    /// # Arguments
    ///
    /// * `path` — destination; parent dirs are created as needed.
    ///
    /// # Errors
    ///
    /// * [`CatalogError::Io`] — create/write failure.
    /// * [`CatalogError::InvalidConfig`] — encode failure.
    /// * [`CatalogError::UnsupportedConfigFormat`] — other extension.
    pub fn save(&self, path: &Path) -> Result<(), CatalogError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CatalogError::io(parent, e))?;
        }
        let bytes = encode_config_bytes(path, self)?;
        fs::write(path, bytes).map_err(|e| CatalogError::io(path, e))
    }

    /// Clear overrides and re-run discovery (Settings “Reset to auto-detect”).
    pub fn reset_to_auto_detect(&mut self) {
        self.game_dir = None;
        self.defs_blob = None;
        self.save_dirs.clear();
        self.tokens_path = None;
        self.auto_detect = true;
        self.apply_auto_detect();
    }

    /// Fill missing or invalid `game_dir` / empty `save_dirs` from defaults.
    ///
    /// Does not overwrite a valid `game_dir` or a non-empty `save_dirs` list.
    /// Tokens and `defs_blob` are never auto-filled.
    pub fn apply_auto_detect(&mut self) {
        let game_ok = self.game_dir.as_ref().is_some_and(|p| is_valid_game_dir(p));
        if !game_ok {
            self.game_dir = detect_game_dir();
        }
        if self.save_dirs.is_empty() {
            self.save_dirs = detect_save_roots().into_iter().map(|r| r.path).collect();
        }
    }

    /// Save roots with inferred [`crate::SaveLocation`] tags for catalog refresh.
    pub fn save_roots(&self) -> Vec<SaveRoot> {
        roots_from_paths(self.save_dirs.iter().cloned())
    }
}

/// Resolve which config file to use under `app_data` (prefer existing, else TOML).
///
/// Preference: existing `config.toml`, else existing `config.json`, else
/// `config.toml` (not yet created).
///
/// # Arguments
///
/// * `app_data` — directory from [`crate::app_data_dir`].
pub fn resolve_config_path(app_data: &Path) -> PathBuf {
    let toml = app_data.join(CONFIG_FILE_TOML);
    let json = app_data.join(CONFIG_FILE_JSON);
    if toml.exists() {
        toml
    } else if json.exists() {
        json
    } else {
        toml
    }
}

fn parse_config_bytes(path: &Path, bytes: &[u8]) -> Result<AppConfig, CatalogError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "toml" => {
            let text =
                std::str::from_utf8(bytes).map_err(|source| CatalogError::InvalidConfig {
                    path: path.to_path_buf(),
                    source: Box::new(source),
                })?;
            toml::from_str(text).map_err(|source| CatalogError::InvalidConfig {
                path: path.to_path_buf(),
                source: Box::new(source),
            })
        }
        "json" => serde_json::from_slice(bytes).map_err(|source| CatalogError::InvalidConfig {
            path: path.to_path_buf(),
            source: Box::new(source),
        }),
        _ => Err(CatalogError::UnsupportedConfigFormat(path.to_path_buf())),
    }
}

fn encode_config_bytes(path: &Path, cfg: &AppConfig) -> Result<Vec<u8>, CatalogError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "toml" => {
            let text =
                toml::to_string_pretty(cfg).map_err(|source| CatalogError::InvalidConfig {
                    path: path.to_path_buf(),
                    source: Box::new(source),
                })?;
            Ok(text.into_bytes())
        }
        "json" => serde_json::to_vec_pretty(cfg).map_err(|source| CatalogError::InvalidConfig {
            path: path.to_path_buf(),
            source: Box::new(source),
        }),
        _ => Err(CatalogError::UnsupportedConfigFormat(path.to_path_buf())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn round_trips_toml_and_json() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = AppConfig {
            game_dir: Some(tmp.path().join("game")),
            defs_blob: Some(tmp.path().join("defs.postcard")),
            save_dirs: vec![tmp.path().join("saves")],
            tokens_path: None,
            auto_detect: false,
        };
        // Avoid filling from real machine paths.
        cfg.auto_detect = false;

        let toml_path = tmp.path().join("config.toml");
        cfg.save(&toml_path).unwrap();
        let loaded = AppConfig::load(&toml_path).unwrap();
        assert_eq!(loaded.game_dir, cfg.game_dir);
        assert_eq!(loaded.save_dirs, cfg.save_dirs);
        assert!(!loaded.auto_detect);

        let json_path = tmp.path().join("config.json");
        cfg.save(&json_path).unwrap();
        let loaded_json = AppConfig::load(&json_path).unwrap();
        assert_eq!(loaded_json.defs_blob, cfg.defs_blob);
    }

    #[test]
    fn missing_file_returns_defaults_without_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("missing.toml");
        // auto_detect may pick up real dirs; just ensure load succeeds.
        let cfg = AppConfig::load(&path).unwrap();
        assert!(cfg.auto_detect);
    }

    #[test]
    fn auto_detect_fills_empty_save_dirs_from_fake_layout() {
        // Drive detection via explicit apply after planting dirs is covered in
        // paths tests; here we only check override preservation.
        let tmp = tempfile::tempdir().unwrap();
        let saves = tmp.path().join("save games");
        fs::create_dir_all(&saves).unwrap();
        let mut cfg = AppConfig {
            save_dirs: vec![saves.clone()],
            auto_detect: true,
            ..AppConfig::default()
        };
        cfg.apply_auto_detect();
        assert_eq!(cfg.save_dirs, vec![saves]);
    }

    #[test]
    fn resolve_config_path_prefers_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let json = tmp.path().join(CONFIG_FILE_JSON);
        fs::write(&json, b"{}").unwrap();
        assert_eq!(resolve_config_path(tmp.path()), json);
    }
}
