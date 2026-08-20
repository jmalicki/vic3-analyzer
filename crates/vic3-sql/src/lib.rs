//! Read-only DataFusion SQL over a loaded campaign (`docs/sql.md`).
//!
//! Wave 2b: fact-table providers + `query(sql) → rows`. Catalog/`use_save` and
//! rich UDFs land in later waves; scalar/TVF registration is an empty stub.

mod binding;
mod error;
mod exec;
mod filter;
mod providers;
mod readonly;
mod schema;
mod session;

pub use binding::SessionBinding;
pub use error::SqlError;
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
