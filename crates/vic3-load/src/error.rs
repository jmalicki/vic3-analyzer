use std::io;
use vic3save::{Vic3Error, Vic3ErrorKind};

/// Failure while loading a Vic3 save or token map.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// Binary (ironman) save encountered without a token map.
    ///
    /// Text saves do not need tokens. Supply a user-owned map via `VIC3_TOKENS`
    /// (or [`crate::load_tokens_slice`]); this project never redistributes
    /// Paradox tokens.
    #[error(
        "binary save requires a token map (VIC3_TOKENS); none was provided (not a serde parse failure)"
    )]
    MissingTokens,

    /// Envelope, melt, or deserialize error from jomini / vic3save.
    #[error(transparent)]
    Vic3(Vic3Error),

    /// Token map text could not be parsed.
    #[error("failed to parse token map: {0}")]
    Tokens(String),

    /// Filesystem error while opening a save or token file.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

impl From<Vic3Error> for LoadError {
    fn from(err: Vic3Error) -> Self {
        match err.kind() {
            Vic3ErrorKind::NoTokens => LoadError::MissingTokens,
            _ => LoadError::Vic3(err),
        }
    }
}

impl From<vic3save::EnvelopeError> for LoadError {
    fn from(err: vic3save::EnvelopeError) -> Self {
        LoadError::from(Vic3Error::from(err))
    }
}
