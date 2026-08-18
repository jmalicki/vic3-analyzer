use thiserror::Error;

/// Failure in the bytes-in JSON-out facade.
#[derive(Debug, Error)]
pub enum WasmError {
    /// Save or token map could not be loaded.
    #[error(transparent)]
    Load(#[from] vic3_load::LoadError),
    /// Plaintext save could not be patched.
    #[error(transparent)]
    Export(#[from] vic3_load::ExportError),
    /// Defs postcard blob could not be decoded.
    #[error(transparent)]
    Defs(#[from] vic3_defs::DefsError),
    /// Option JSON could not be parsed.
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A browser definitions-file manifest referenced bytes outside its payload.
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
    /// A worker analysis method ran before a save and definitions were loaded.
    #[error("no analysis is loaded")]
    NoLoadedAnalysis,
}
