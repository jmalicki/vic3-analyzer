//! Platform path helpers: app data, game install, Paradox/Steam save roots.
//!
//! Auto-detect only walks known Steam / Paradox layouts (`docs/desktop.md`).
//! Configured `save_dirs` are allowlisted as-is; [`infer_location`] tags Steam
//! Cloud paths for stub disambiguation.

use std::path::{Path, PathBuf};

use crate::error::CatalogError;
use crate::location::SaveLocation;

/// Application id used under the platform data directory.
pub const APP_NAME: &str = "vic3-analyzer";

/// Default config filename (TOML). JSON (`config.json`) is also accepted.
pub const CONFIG_FILE_TOML: &str = "config.toml";
/// Alternate config filename accepted by [`crate::AppConfig::load`].
pub const CONFIG_FILE_JSON: &str = "config.json";

/// Victoria 3 Steam app id (for Cloud cache under `userdata/<id>/529340/`).
pub const VIC3_STEAM_APP_ID: &str = "529340";

/// Resolve the shared app data directory (`…/vic3-analyzer`).
///
/// Honors `XDG_DATA_HOME` first (same convention as the CLI archive), then
/// [`dirs::data_local_dir`].
///
/// # Errors
///
/// [`CatalogError::NoAppDataDir`] when neither source yields a path.
pub fn app_data_dir() -> Result<PathBuf, CatalogError> {
    if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(root).join(APP_NAME));
    }
    dirs::data_local_dir()
        .map(|root| root.join(APP_NAME))
        .ok_or(CatalogError::NoAppDataDir)
}

/// Default path for the shared config file (`config.toml` under app data).
///
/// # Errors
///
/// Propagates [`CatalogError::NoAppDataDir`] from [`app_data_dir`].
pub fn default_config_path() -> Result<PathBuf, CatalogError> {
    Ok(app_data_dir()?.join(CONFIG_FILE_TOML))
}

/// Candidate absolute paths to `…/Victoria 3/game` for the current OS.
///
/// Order is preference order for [`detect_game_dir`]. Not all candidates exist.
pub fn default_game_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            out.push(
                home.join("Library/Application Support/Steam/steamapps/common/Victoria 3/game"),
            );
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(home) = dirs::home_dir() {
            out.push(home.join("AppData/Local/Steam/steamapps/common/Victoria 3/game"));
        }
        out.push(PathBuf::from(
            r"C:\Program Files (x86)\Steam\steamapps\common\Victoria 3\game",
        ));
        out.push(PathBuf::from(
            r"C:\Program Files\Steam\steamapps\common\Victoria 3\game",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            out.push(home.join(".local/share/Steam/steamapps/common/Victoria 3/game"));
            out.push(home.join(".steam/steam/steamapps/common/Victoria 3/game"));
            out.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/Victoria 3/game"));
        }
    }
    out
}

/// True when `dir` looks like a Vic3 `game/` tree (contains `common/`).
pub fn is_valid_game_dir(dir: &Path) -> bool {
    dir.is_dir() && dir.join("common").is_dir()
}

/// First existing valid candidate from [`default_game_dir_candidates`].
pub fn detect_game_dir() -> Option<PathBuf> {
    default_game_dir_candidates()
        .into_iter()
        .find(|p| is_valid_game_dir(p))
}

/// Default local Paradox Documents save root for this OS (may not exist yet).
pub fn default_local_save_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::document_dir().map(|d| d.join("Paradox Interactive/Victoria 3/save games"))
    }
    #[cfg(target_os = "windows")]
    {
        dirs::document_dir().map(|d| d.join(r"Paradox Interactive\Victoria 3\save games"))
    }
    #[cfg(target_os = "linux")]
    {
        dirs::data_local_dir().map(|d| d.join("Paradox Interactive/Victoria 3/save games"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// Steam library roots that may contain `userdata/` (allowlisted patterns only).
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Library/Application Support/Steam"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("AppData/Local/Steam"));
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".local/share/Steam"));
            roots.push(home.join(".steam/steam"));
            roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        }
    }
    roots
}

/// Discover Steam Cloud save roots: `userdata/<id>/529340/remote/save games`.
///
/// Only directories that currently exist are returned (deduped, sorted).
pub fn detect_steam_cloud_save_dirs() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for steam in steam_roots() {
        let userdata = steam.join("userdata");
        let Ok(entries) = std::fs::read_dir(&userdata) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let save_games = path
                .join(VIC3_STEAM_APP_ID)
                .join("remote")
                .join("save games");
            if save_games.is_dir() {
                found.push(save_games);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// A save-catalog root with its agent-facing `location` tag.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SaveRoot {
    /// Absolute directory to scan for top-level `*.v3`.
    pub path: PathBuf,
    /// Tag written into catalog / SQL `location`.
    pub location: SaveLocation,
}

/// Auto-detect local + Steam Cloud roots that currently exist on disk.
pub fn detect_save_roots() -> Vec<SaveRoot> {
    let mut roots = Vec::new();
    if let Some(local) = default_local_save_dir() {
        if local.is_dir() {
            roots.push(SaveRoot {
                path: local,
                location: SaveLocation::Local,
            });
        }
    }
    for path in detect_steam_cloud_save_dirs() {
        roots.push(SaveRoot {
            path,
            location: SaveLocation::SteamCloud,
        });
    }
    roots
}

/// Infer `location` for a configured absolute path (Steam Cloud pattern → cloud).
///
/// Matches `/userdata/` + `/{VIC3_STEAM_APP_ID}/` in a normalized path string.
pub fn infer_location(path: &Path) -> SaveLocation {
    let s = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if s.contains("/userdata/") && s.contains(&format!("/{VIC3_STEAM_APP_ID}/")) {
        SaveLocation::SteamCloud
    } else {
        SaveLocation::Local
    }
}

/// Build [`SaveRoot`]s from absolute directory paths (config `save_dirs`).
///
/// # Arguments
///
/// * `paths` — absolute save-root directories from [`crate::AppConfig::save_dirs`].
pub fn roots_from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<SaveRoot> {
    paths
        .into_iter()
        .map(|path| {
            let location = infer_location(&path);
            SaveRoot { path, location }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn validates_game_dir_by_common() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        fs::create_dir_all(game.join("common")).unwrap();
        assert!(is_valid_game_dir(&game));
        assert!(!is_valid_game_dir(tmp.path()));
    }

    #[test]
    fn infers_steam_cloud_location() {
        let p = PathBuf::from("/home/u/.local/share/Steam/userdata/123/529340/remote/save games");
        assert_eq!(infer_location(&p), SaveLocation::SteamCloud);
        let local = PathBuf::from("/home/u/Documents/Paradox Interactive/Victoria 3/save games");
        assert_eq!(infer_location(&local), SaveLocation::Local);
    }

    #[test]
    fn app_data_honors_xdg() {
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: test process only; restore not required beyond scope.
        std::env::set_var("XDG_DATA_HOME", tmp.path());
        let dir = app_data_dir().unwrap();
        assert_eq!(dir, tmp.path().join(APP_NAME));
        std::env::remove_var("XDG_DATA_HOME");
    }
}
