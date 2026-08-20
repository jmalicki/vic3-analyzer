//! Host session state: catalog, `use_save`, and `latest.*` cache.
//!
//! Binding is a Rust API — never a mutating `SELECT` (`docs/sql.md`).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use vic3_api::load_analysis_snapshot_from_path;
use vic3_catalog::{SaveCatalog, SaveEntry, SaveLocation, SaveRoot};

use crate::binding::SessionBinding;
use crate::SqlError;

/// How the engine loads defs/tokens when resolving a save path.
#[derive(Debug, Clone)]
pub struct EngineLoadOpts {
    /// Postcard defs blob path (game install blob or fixture).
    pub defs_blob: PathBuf,
    /// Optional token map for binary/ironman saves.
    pub tokens: Option<PathBuf>,
    /// JSON `SolveOpts` (empty / `{}` = defaults).
    pub solve_opts_json: String,
}

impl EngineLoadOpts {
    pub fn new(defs_blob: impl Into<PathBuf>) -> Self {
        Self {
            defs_blob: defs_blob.into(),
            tokens: None,
            solve_opts_json: "{}".to_string(),
        }
    }

    pub fn with_tokens(mut self, tokens: impl Into<PathBuf>) -> Self {
        self.tokens = Some(tokens.into());
        self
    }
}

/// Host `use_save` arguments (`docs/mcp.md`) — Rust API, not SQL.
#[derive(Debug, Clone, Default)]
pub struct UseSaveRequest {
    /// Filename stub (`autosave` or `autosave.v3`).
    pub name: Option<String>,
    /// `latest` | `latest_autosave` | `latest_named`.
    pub selector: Option<String>,
    /// Disambiguate stub: `local` | `steam_cloud`.
    pub location: Option<SaveLocation>,
    /// Further disambiguation when the same stub appears twice.
    pub mtime: Option<SystemTime>,
}

/// Result of a successful [`crate::SqlEngine::use_save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseSaveResult {
    pub name: String,
    pub kind: String,
    pub in_game_date: Option<String>,
    pub country: Option<String>,
    pub loaded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveMeta {
    pub entry: SaveEntry,
    pub binding: Arc<SessionBinding>,
}

#[derive(Debug)]
struct LatestCache {
    entry: SaveEntry,
    binding: Arc<SessionBinding>,
}

/// Shared host state behind `saves` / `latest.*` / `use_save`.
#[derive(Debug)]
pub(crate) struct HostState {
    catalog: RwLock<SaveCatalog>,
    active: RwLock<Option<ActiveMeta>>,
    latest: RwLock<Option<LatestCache>>,
    load: EngineLoadOpts,
}

impl HostState {
    pub(crate) fn new(catalog: SaveCatalog, load: EngineLoadOpts) -> Self {
        Self {
            catalog: RwLock::new(catalog),
            active: RwLock::new(None),
            latest: RwLock::new(None),
            load,
        }
    }

    pub(crate) fn catalog_entries(&self) -> Vec<SaveEntry> {
        self.catalog
            .read()
            .expect("catalog lock")
            .entries()
            .to_vec()
    }

    pub(crate) fn active_meta(&self) -> Option<ActiveMeta> {
        self.active.read().expect("active lock").clone()
    }

    pub(crate) fn active_binding(&self) -> Option<Arc<SessionBinding>> {
        self.active
            .read()
            .expect("active lock")
            .as_ref()
            .map(|a| Arc::clone(&a.binding))
    }

    pub(crate) fn set_active(&self, meta: ActiveMeta) {
        *self.active.write().expect("active lock") = Some(meta);
    }

    pub(crate) fn refresh_catalog(&self, roots: &[SaveRoot]) -> Result<usize, SqlError> {
        let catalog = SaveCatalog::refresh(roots)?;
        let n = catalog.len();
        *self.catalog.write().expect("catalog lock") = catalog;
        *self.latest.write().expect("latest lock") = None;
        Ok(n)
    }

    pub(crate) fn resolve_request(&self, req: &UseSaveRequest) -> Result<SaveEntry, SqlError> {
        let catalog = self.catalog.read().expect("catalog lock");
        match (&req.name, &req.selector) {
            (Some(name), None) => catalog
                .resolve(name, req.location, req.mtime)
                .cloned()
                .map_err(SqlError::from_catalog_resolve),
            (None, Some(selector)) => catalog
                .select(selector)
                .cloned()
                .map_err(SqlError::from_catalog_resolve),
            (Some(_), Some(_)) => Err(SqlError::internal(
                "use_save: provide exactly one of name or selector",
            )),
            (None, None) => Err(SqlError::internal(
                "use_save: provide name or selector (latest|latest_autosave|latest_named)",
            )),
        }
    }

    pub(crate) fn load_entry(
        &self,
        entry: &SaveEntry,
        install: bool,
    ) -> Result<LoadedSave, SqlError> {
        let path = entry.path.as_ref().ok_or_else(|| {
            SqlError::internal(format!("catalog entry {} has no path", entry.name))
        })?;
        let snap = load_analysis_snapshot_from_path(
            path,
            self.load.tokens.as_deref(),
            &self.load.defs_blob,
            &self.load.solve_opts_json,
            install,
        )?;
        Ok(LoadedSave {
            binding: Arc::new(SessionBinding::new(snap.defs, snap.world, snap.prices)),
            in_game_date: snap.date,
            country: snap.tag,
        })
    }

    /// Ensure `latest.*` binding for the current catalog max-mtime save.
    ///
    /// Does **not** change the active session (`install = false`).
    pub(crate) fn ensure_latest_binding(&self) -> Result<Arc<SessionBinding>, SqlError> {
        let entry = {
            let catalog = self.catalog.read().expect("catalog lock");
            catalog
                .select("latest")
                .cloned()
                .map_err(SqlError::from_catalog_resolve)?
        };

        {
            let cache = self.latest.read().expect("latest lock");
            if let Some(cached) = cache.as_ref() {
                if same_entry(&cached.entry, &entry) {
                    return Ok(Arc::clone(&cached.binding));
                }
            }
        }

        let loaded = self.load_entry(&entry, false)?;
        let binding = Arc::clone(&loaded.binding);
        *self.latest.write().expect("latest lock") = Some(LatestCache {
            entry,
            binding: Arc::clone(&binding),
        });
        Ok(binding)
    }
}

/// Outcome of loading one catalog entry through `vic3-api`.
pub(crate) struct LoadedSave {
    pub binding: Arc<SessionBinding>,
    pub in_game_date: Option<String>,
    pub country: Option<String>,
}

fn same_entry(a: &SaveEntry, b: &SaveEntry) -> bool {
    a.name == b.name && a.location == b.location && a.mtime == b.mtime && a.path == b.path
}
