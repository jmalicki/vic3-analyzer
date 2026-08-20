use datafusion::error::DataFusionError;
use thiserror::Error;
use vic3_api::ApiError;
use vic3_catalog::{CatalogError, SaveEntry, SaveKind, SaveLocation};

/// Agent-facing ambiguous-stub candidate (`docs/sql.md` / `docs/mcp.md`).
///
/// Strips the catalog's absolute `path` so MCP / SQL hosts can surface choices
/// without leaking filesystem layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveCandidate {
    /// Filename stub (primary handle).
    pub name: String,
    pub kind: SaveKind,
    /// Filesystem mtime (disambiguator when the same stub appears twice).
    pub mtime: std::time::SystemTime,
    /// `local` | `steam_cloud`.
    pub location: SaveLocation,
}

impl From<&SaveEntry> for SaveCandidate {
    fn from(entry: &SaveEntry) -> Self {
        Self {
            name: entry.name.clone(),
            kind: entry.kind,
            mtime: entry.mtime,
            location: entry.location,
        }
    }
}

/// Errors from the SQL façade.
#[derive(Debug, Error)]
pub enum SqlError {
    /// Statement rejected by the read-only gate (DDL/DML, etc.).
    #[error("read-only SQL rejected: {0}")]
    ReadOnly(String),
    /// `active.*` / unqualified facts queried before `use_save` / `bind`.
    #[error("no active session binding")]
    Unbound,
    /// No catalog entry matches the requested stub / selector.
    #[error("save not found: {0}")]
    NotFound(String),
    /// Same stub matches multiple roots; callers must pass `location` / `mtime`.
    #[error(
        "ambiguous save stub {stub:?}: matches {} candidates — disambiguate with location/mtime",
        .candidates.len()
    )]
    Ambiguous {
        stub: String,
        candidates: Vec<SaveCandidate>,
    },
    /// Other catalog failures (I/O, config) not remapped to stub resolution.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error(transparent)]
    DataFusion(#[from] DataFusionError),
    #[error("{0}")]
    Internal(String),
}

impl SqlError {
    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub(crate) fn read_only(msg: impl Into<String>) -> Self {
        Self::ReadOnly(msg.into())
    }

    /// Map catalog stub/selector failures to agent-facing [`Self::Ambiguous`] /
    /// [`Self::NotFound`] (path-free candidates); leave other catalog errors as
    /// [`Self::Catalog`].
    pub(crate) fn from_catalog_resolve(err: CatalogError) -> Self {
        match err {
            CatalogError::Ambiguous { stub, candidates } => Self::Ambiguous {
                stub,
                candidates: candidates.iter().map(SaveCandidate::from).collect(),
            },
            CatalogError::NotFound(msg) => Self::NotFound(msg),
            other => Self::Catalog(other),
        }
    }
}
