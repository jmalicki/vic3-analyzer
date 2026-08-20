use datafusion::error::DataFusionError;
use thiserror::Error;

/// Errors from the SQL façade.
#[derive(Debug, Error)]
pub enum SqlError {
    #[error("read-only SQL rejected: {0}")]
    ReadOnly(String),
    #[error("no active session binding")]
    Unbound,
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
}
