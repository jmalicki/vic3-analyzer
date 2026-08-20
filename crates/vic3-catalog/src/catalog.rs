//! Scan allowlisted save roots into an agent-facing catalog.
//!
//! Non-recursive: top-level `*.v3` only. Sorted newest-mtime first. Absolute
//! paths stay on [`SaveEntry::path`] for loaders; strip them before MCP/SQL
//! agent payloads.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::CatalogError;
use crate::location::SaveLocation;
use crate::paths::SaveRoot;
use crate::stub::{classify_kind, normalize_stub, SaveKind};

/// One row of the save catalog (agent-facing fields + internal absolute path).
///
/// Agent-facing columns match `docs/sql.md` `saves` table. `path` is for Rust
/// loaders only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveEntry {
    /// Filename stub (primary handle).
    pub name: String,
    pub kind: SaveKind,
    /// Filesystem mtime.
    #[serde(serialize_with = "ser_mtime", deserialize_with = "de_mtime")]
    pub mtime: SystemTime,
    /// Cheap metadata when known; null after a scan-only refresh.
    pub in_game_date: Option<String>,
    pub country: Option<String>,
    pub location: SaveLocation,
    /// Absolute path — Rust-only; not an agent-facing SQL column by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

fn ser_mtime<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    s.serialize_f64(secs)
}

fn de_mtime<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
    let secs = f64::deserialize(d)?;
    Ok(UNIX_EPOCH + Duration::from_secs_f64(secs.max(0.0)))
}

impl SaveEntry {
    fn from_file(path: &Path, location: SaveLocation) -> Result<Self, CatalogError> {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| CatalogError::NotFound(path.display().to_string()))?;
        let name = normalize_stub(file_name)
            .map_err(|_| CatalogError::NotFound(format!("invalid save filename: {file_name}")))?;
        let meta = fs::metadata(path).map_err(|e| CatalogError::io(path, e))?;
        let mtime = meta.modified().map_err(|e| CatalogError::io(path, e))?;
        Ok(Self {
            kind: classify_kind(&name),
            name,
            mtime,
            in_game_date: None,
            country: None,
            location,
            path: Some(path.to_path_buf()),
        })
    }
}

/// In-memory catalog snapshot.
///
/// Built by [`scan_roots`] / [`SaveCatalog::refresh`]. Fed into
/// `vic3-sql::SqlEngine` as the `saves` table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveCatalog {
    entries: Vec<SaveEntry>,
}

impl SaveCatalog {
    /// Wrap an already-built entry list (tests / SQL providers).
    pub fn new(entries: Vec<SaveEntry>) -> Self {
        Self { entries }
    }

    /// Borrow all entries (newest mtime first after a scan).
    pub fn entries(&self) -> &[SaveEntry] {
        &self.entries
    }

    /// Consume into the underlying entry list.
    pub fn into_entries(self) -> Vec<SaveEntry> {
        self.entries
    }

    /// Number of catalog rows.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no saves were found under the configured roots.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Rescan allowlisted roots (non-recursive; `*.v3` files only).
    ///
    /// # Errors
    ///
    /// Propagates [`CatalogError::Io`] from [`scan_roots`].
    pub fn refresh(roots: &[SaveRoot]) -> Result<Self, CatalogError> {
        scan_roots(roots)
    }

    /// Look up by stub; optional `location` / `mtime` disambiguators.
    ///
    /// # Arguments
    ///
    /// * `name` — stub or `*.v3` basename (normalized via [`normalize_stub`]).
    /// * `location` — when set, only that [`SaveLocation`] matches.
    /// * `mtime` — when set, exact filesystem mtime match.
    ///
    /// # Errors
    ///
    /// * [`CatalogError::NotFound`] — zero matches (or invalid stub).
    /// * [`CatalogError::Ambiguous`] — multiple matches; `candidates` listed.
    pub fn resolve(
        &self,
        name: &str,
        location: Option<SaveLocation>,
        mtime: Option<SystemTime>,
    ) -> Result<&SaveEntry, CatalogError> {
        let stub = normalize_stub(name).map_err(|e| CatalogError::NotFound(e.to_string()))?;
        let mut matches: Vec<&SaveEntry> = self
            .entries
            .iter()
            .filter(|e| e.name == stub)
            .filter(|e| location.is_none_or(|loc| e.location == loc))
            .filter(|e| mtime.is_none_or(|t| e.mtime == t))
            .collect();

        match matches.len() {
            0 => Err(CatalogError::NotFound(stub)),
            1 => Ok(matches.remove(0)),
            _ => Err(CatalogError::Ambiguous {
                stub,
                candidates: matches.into_iter().cloned().collect(),
            }),
        }
    }

    /// Host selectors: `latest`, `latest_autosave`, `latest_named`.
    ///
    /// Picks the newest matching entry by filesystem `mtime` (not in-game date).
    ///
    /// # Arguments
    ///
    /// * `selector` — one of the three stable names above.
    ///
    /// # Errors
    ///
    /// [`CatalogError::NotFound`] for unknown selectors or empty filtered set.
    pub fn select(&self, selector: &str) -> Result<&SaveEntry, CatalogError> {
        let filter: Box<dyn Fn(&SaveEntry) -> bool> = match selector {
            "latest" => Box::new(|_| true),
            "latest_autosave" => Box::new(|e| e.kind == SaveKind::Autosave),
            "latest_named" => Box::new(|e| e.kind == SaveKind::Named),
            other => {
                return Err(CatalogError::NotFound(format!("unknown selector: {other}")));
            }
        };
        self.entries
            .iter()
            .filter(|e| filter(e))
            .max_by_key(|e| e.mtime)
            .ok_or_else(|| CatalogError::NotFound(selector.to_string()))
    }
}

/// Scan each root for top-level `*.v3` files.
///
/// Missing root directories are skipped (not an error). Path-like basenames are
/// skipped. Results sorted by mtime descending, then name.
///
/// # Arguments
///
/// * `roots` — allowlisted directories from [`crate::AppConfig::save_roots`].
///
/// # Errors
///
/// [`CatalogError::Io`] when a root exists but cannot be read, or a file’s
/// metadata cannot be read.
pub fn scan_roots(roots: &[SaveRoot]) -> Result<SaveCatalog, CatalogError> {
    let mut entries = Vec::new();
    for root in roots {
        if !root.path.is_dir() {
            continue;
        }
        let read = fs::read_dir(&root.path).map_err(|e| CatalogError::io(&root.path, e))?;
        for entry in read {
            let entry = entry.map_err(|e| CatalogError::io(&root.path, e))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.to_ascii_lowercase().ends_with(".v3") {
                continue;
            }
            // Skip path-like basenames (should not happen for real files).
            if normalize_stub(name).is_err() {
                continue;
            }
            entries.push(SaveEntry::from_file(&path, root.location)?);
        }
    }
    entries.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.name.cmp(&b.name)));
    Ok(SaveCatalog { entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::SaveRoot;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    fn write_save(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"SAV").unwrap();
    }

    #[test]
    fn scans_local_and_cloud_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("local");
        let cloud = tmp.path().join("cloud");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&cloud).unwrap();
        write_save(&local, "autosave.v3");
        write_save(&cloud, "Campaign.v3");
        fs::write(local.join("notes.txt"), b"x").unwrap();

        let catalog = scan_roots(&[
            SaveRoot {
                path: local,
                location: SaveLocation::Local,
            },
            SaveRoot {
                path: cloud,
                location: SaveLocation::SteamCloud,
            },
        ])
        .unwrap();

        assert_eq!(catalog.len(), 2);
        let auto = catalog.resolve("autosave", None, None).unwrap();
        assert_eq!(auto.kind, SaveKind::Autosave);
        assert_eq!(auto.location, SaveLocation::Local);
        let named = catalog.resolve("Campaign.v3", None, None).unwrap();
        assert_eq!(named.kind, SaveKind::Named);
        assert_eq!(named.location, SaveLocation::SteamCloud);
    }

    #[test]
    fn ambiguous_stub_errors_with_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("local");
        let cloud = tmp.path().join("cloud");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&cloud).unwrap();
        write_save(&local, "autosave.v3");
        write_save(&cloud, "autosave.v3");

        let catalog = scan_roots(&[
            SaveRoot {
                path: local,
                location: SaveLocation::Local,
            },
            SaveRoot {
                path: cloud,
                location: SaveLocation::SteamCloud,
            },
        ])
        .unwrap();

        let err = catalog.resolve("autosave", None, None).unwrap_err();
        match err {
            CatalogError::Ambiguous { stub, candidates } => {
                assert_eq!(stub, "autosave");
                assert_eq!(candidates.len(), 2);
                let locs: Vec<_> = candidates.iter().map(|c| c.location).collect();
                assert!(locs.contains(&SaveLocation::Local));
                assert!(locs.contains(&SaveLocation::SteamCloud));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }

        let disambig = catalog
            .resolve("autosave", Some(SaveLocation::SteamCloud), None)
            .unwrap();
        assert_eq!(disambig.location, SaveLocation::SteamCloud);
    }

    #[test]
    fn selectors_pick_latest_by_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("saves");
        fs::create_dir_all(&dir).unwrap();
        write_save(&dir, "old.v3");
        thread::sleep(Duration::from_millis(20));
        write_save(&dir, "autosave.v3");
        thread::sleep(Duration::from_millis(20));
        write_save(&dir, "new_named.v3");

        let catalog = scan_roots(&[SaveRoot {
            path: dir,
            location: SaveLocation::Local,
        }])
        .unwrap();

        assert_eq!(catalog.select("latest").unwrap().name, "new_named");
        assert_eq!(catalog.select("latest_autosave").unwrap().name, "autosave");
        assert_eq!(catalog.select("latest_named").unwrap().name, "new_named");
    }
}
