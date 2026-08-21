use std::path::PathBuf;
use thiserror::Error;

/// Failure in the transport-free analysis API.
///
/// Hosts map this to CLI exit messages, wasm `JsError`, Tauri invoke errors, or
/// MCP tool errors via [`std::fmt::Display`] (stable, user-facing strings).
///
/// Session methods return [`Self::NoLoadedAnalysis`] when nothing is installed.
/// Binary saves without a token map surface as [`Self::Load`]
/// (`MissingTokens`), not a serde mystery.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Save or token map could not be loaded.
    #[error(transparent)]
    Load(#[from] vic3_load::LoadError),
    /// Plaintext save could not be patched.
    #[error(transparent)]
    Export(#[from] vic3_load::ExportError),
    /// Defs postcard blob could not be decoded or built.
    #[error(transparent)]
    Defs(#[from] vic3_defs::DefsError),
    /// Filesystem read failed for a path-based loader.
    #[error("failed to read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Option / delta / patch JSON could not be parsed.
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A definitions-file manifest referenced bytes outside its payload.
    #[error("invalid definitions file manifest: {0}")]
    DefsManifest(String),
    /// Goal DSL could not be compiled.
    #[error(transparent)]
    Goal(#[from] vic3_planning::GoalError),
    /// Save projection could not be built.
    #[error(transparent)]
    World(#[from] vic3_planning::WorldError),
    /// No plan fits the current model and limits.
    #[error(transparent)]
    Plan(#[from] vic3_planning::PlanError),
    /// The save contains no playable country (gaps / plan need a player tag).
    #[error("save has no playable country")]
    NoCountry,
    /// A `loaded_*` method ran before [`crate::load_analysis_json`] (or snapshot install).
    #[error("no analysis is loaded")]
    NoLoadedAnalysis,
    /// Desktop config / defs path resolution failed (GUI Settings and MCP share this).
    #[error("{0}")]
    Config(String),
}

impl ApiError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
