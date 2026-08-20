use std::io;
use std::path::PathBuf;

use crate::catalog::SaveEntry;

/// Failure while discovering paths, loading config, or resolving a stub.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// Same filename stub appears under more than one allowlisted root.
    #[error("ambiguous save stub {stub:?}: matches {} candidates", .candidates.len())]
    Ambiguous {
        stub: String,
        candidates: Vec<SaveEntry>,
    },

    /// No catalog entry matches the requested stub / selector.
    #[error("save not found: {0}")]
    NotFound(String),

    /// Config file could not be parsed.
    #[error("invalid config at {}: {source}", .path.display())]
    InvalidConfig {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Unsupported config extension (expect `.toml` or `.json`).
    #[error("unsupported config format at {} (use .toml or .json)", .0.display())]
    UnsupportedConfigFormat(PathBuf),

    /// Filesystem error.
    #[error("I/O error{}: {source}", .path.as_ref().map(|p| format!(" at {}", p.display())).unwrap_or_default())]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: io::Error,
    },

    /// Could not resolve a platform app-data directory.
    #[error("could not determine app data directory")]
    NoAppDataDir,
}

impl CatalogError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: Some(path.into()),
            source,
        }
    }
}
