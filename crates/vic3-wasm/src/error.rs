use thiserror::Error;

/// Failure in the bytes-in JSON-out facade.
#[derive(Debug, Error)]
pub enum WasmError {
    /// Save or token map could not be loaded.
    #[error(transparent)]
    Load(#[from] vic3_load::LoadError),
    /// Defs postcard blob could not be decoded.
    #[error(transparent)]
    Defs(#[from] vic3_defs::DefsError),
    /// Option JSON could not be parsed.
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}
