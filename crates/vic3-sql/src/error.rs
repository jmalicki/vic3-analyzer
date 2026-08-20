use datafusion::error::DataFusionError;
use thiserror::Error;
use vic3_api::ApiError;
use vic3_catalog::{CatalogError, SaveEntry, SaveKind, SaveLocation};

/// Agent-facing ambiguous-stub candidate (`docs/sql.md` / `docs/mcp.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveCandidate {
    pub name: String,
    pub kind: SaveKind,
    pub mtime: std::time::SystemTime,
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
    #[error("read-only SQL rejected: {0}")]
    ReadOnly(String),
    #[error("no active session binding")]
    Unbound,
    #[error("save not found: {0}")]
    NotFound(String),
    #[error(
        "ambiguous save stub {stub:?}: matches {} candidates — disambiguate with location/mtime",
        .candidates.len()
    )]
    Ambiguous {
        stub: String,
        candidates: Vec<SaveCandidate>,
    },
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
