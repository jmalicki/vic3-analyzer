use std::path::PathBuf;
use thiserror::Error;

/// Failure in the transport-free analysis API.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Save or token map could not be loaded.
    #[error(transparent)]
    Load(#[from] vic3_load::LoadError),
    /// Plaintext save could not be patched.
    #[error(transparent)]
    Export(#[from] vic3_load::ExportError),
    /// Defs postcard blob could not be decoded.
    #[error(transparent)]
    Defs(#[from] vic3_defs::DefsError),
    /// Filesystem read failed for a path-based loader.
    #[error("failed to read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Option JSON could not be parsed.
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A definitions-file manifest referenced bytes outside its payload.
    #[error("invalid definitions file manifest: {0}")]
    DefsManifest(String),
    /// Goal DSL could not be compiled.
    #[error(transparent)]
    Goal(#[from] vic3_goals::GoalError),
    /// Save projection could not be built.
    #[error(transparent)]
    World(#[from] vic3_world::WorldError),
    /// No plan fits the current model and limits.
    #[error(transparent)]
    Plan(#[from] vic3_plan::PlanError),
    /// The save contains no playable country.
    #[error("save has no playable country")]
    NoCountry,
    /// An analysis method ran before a save and definitions were loaded.
    #[error("no analysis is loaded")]
    NoLoadedAnalysis,
}

impl ApiError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
