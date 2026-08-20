//! JSON DTOs for companion UI invokes (stubs / paths as strings).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use vic3_catalog::{AppConfig, SaveEntry, SaveLocation};

/// Serializable app config for Settings round-trips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDto {
    pub game_dir: Option<String>,
    pub defs_blob: Option<String>,
    pub save_dirs: Vec<String>,
    pub tokens_path: Option<String>,
    pub auto_detect: bool,
    /// Absolute path of the config file on disk.
    pub config_path: String,
}

impl ConfigDto {
    pub fn from_config(cfg: &AppConfig, config_path: &std::path::Path) -> Self {
        Self {
            game_dir: cfg.game_dir.as_ref().map(|p| p.display().to_string()),
            defs_blob: cfg.defs_blob.as_ref().map(|p| p.display().to_string()),
            save_dirs: cfg
                .save_dirs
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            tokens_path: cfg.tokens_path.as_ref().map(|p| p.display().to_string()),
            auto_detect: cfg.auto_detect,
            config_path: config_path.display().to_string(),
        }
    }

    pub fn into_config(self) -> AppConfig {
        AppConfig {
            game_dir: nonempty_path(self.game_dir),
            defs_blob: nonempty_path(self.defs_blob),
            save_dirs: self
                .save_dirs
                .into_iter()
                .filter_map(|s| {
                    let t = s.trim();
                    (!t.is_empty()).then(|| PathBuf::from(t))
                })
                .collect(),
            tokens_path: nonempty_path(self.tokens_path),
            auto_detect: self.auto_detect,
        }
    }
}

fn nonempty_path(value: Option<String>) -> Option<PathBuf> {
    value.and_then(|s| {
        let t = s.trim();
        (!t.is_empty()).then(|| PathBuf::from(t))
    })
}

/// Agent/UI-facing save row (no absolute path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveStubDto {
    pub name: String,
    pub kind: String,
    pub location: String,
    pub mtime: f64,
    pub in_game_date: Option<String>,
    pub country: Option<String>,
}

impl From<&SaveEntry> for SaveStubDto {
    fn from(entry: &SaveEntry) -> Self {
        let mtime = entry
            .mtime
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        Self {
            name: entry.name.clone(),
            kind: entry.kind.as_str().to_string(),
            location: entry.location.as_str().to_string(),
            mtime,
            in_game_date: entry.in_game_date.clone(),
            country: entry.country.clone(),
        }
    }
}

/// First-launch / Settings status for the rolling dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardDto {
    pub config: ConfigDto,
    pub game_detected: bool,
    pub save_root_count: usize,
    pub save_count: usize,
    pub loaded_stub: Option<String>,
    pub detection_hints: Vec<String>,
}

/// Optional location filter for `use_save` / resolve.
pub fn parse_location(raw: Option<&str>) -> Result<Option<SaveLocation>, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => s.parse().map(Some),
    }
}
