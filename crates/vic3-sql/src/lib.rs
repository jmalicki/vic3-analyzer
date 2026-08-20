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
pub use providers::{FactTable, FACT_TABLES};
pub use session::{ActiveSessionInfo, SqlEngine};

/// Static schema registry for MCP `vic3://schema` / agent discovery.
///
/// Mirrors fact tables + TVFs from [`docs/sql.md`](../../docs/sql.md); keep in
/// sync with [`schema`] and [`providers::FACT_TABLES`].
pub fn schema_catalog_json() -> serde_json::Value {
    use providers::FACT_TABLES;
    let tables: Vec<serde_json::Value> = FACT_TABLES
        .iter()
        .map(|t| {
            let fields: Vec<serde_json::Value> = t
                .schema()
                .fields()
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "name": f.name(),
                        "data_type": format!("{:?}", f.data_type()),
                        "nullable": f.is_nullable(),
                    })
                })
                .collect();
            serde_json::json!({
                "name": t.name(),
                "schemas": ["active", "latest", "(unqualified after use_save)"],
                "columns": fields,
            })
        })
        .collect();

    let saves_fields: Vec<serde_json::Value> = schema::saves_schema()
        .fields()
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f.name(),
                "data_type": format!("{:?}", f.data_type()),
                "nullable": f.is_nullable(),
            })
        })
        .collect();

    serde_json::json!({
        "catalog_table": {
            "name": "saves",
            "columns": saves_fields,
        },
        "fact_tables": tables,
        "tvfs": [
            {
                "name": "plan",
                "signature": "plan(goal [, max_days [, label]])",
                "columns": schema::plan_schema().fields().iter().map(|f| f.name().to_string()).collect::<Vec<_>>(),
            },
            {
                "name": "gaps",
                "signature": "gaps(goal)",
                "columns": schema::gaps_schema().fields().iter().map(|f| f.name().to_string()).collect::<Vec<_>>(),
            },
        ],
        "notes": [
            "Session binding is host use_save — never SELECT set_active_save(...).",
            "SQL is read-only: SELECT / WITH…SELECT / EXPLAIN only.",
            "Stubs are filename basenames; no filesystem path arguments.",
        ],
    })
}

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
