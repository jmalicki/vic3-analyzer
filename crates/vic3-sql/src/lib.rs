//! Read-only DataFusion SQL over a loaded campaign (`docs/sql.md`).
//!
//! Fact-table providers + `query(sql) → rows`, diagnostics scalars/TVFs
//! (`alerts`, `shortage_analysis`, `good_price`, …), planning TVFs `plan` /
//! `gaps`, catalog table `saves`, host [`SqlEngine::use_save`], and
//! `active.*` / `latest.*` views. Session binding is never a mutating
//! `SELECT`.

mod binding;
mod error;
mod exec;
mod filter;
mod host;
mod providers;
mod readonly;
mod schema;
mod session;
mod udfs;

pub use binding::SessionBinding;
pub use error::{SaveCandidate, SqlError};
pub use host::{EngineLoadOpts, UseSaveRequest, UseSaveResult};
pub use session::SqlEngine;

use datafusion::arrow::array::RecordBatch;

/// Run `sql` against a bound engine on a current-thread Tokio runtime.
///
/// Prefer [`SqlEngine::query`] when already inside an async context.
pub fn query(engine: &SqlEngine, sql: &str) -> Result<Vec<RecordBatch>, SqlError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SqlError::internal(format!("tokio runtime: {e}")))?;
    rt.block_on(engine.query(sql))
}
