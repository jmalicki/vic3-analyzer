//! Read-only DataFusion SQL over a loaded campaign (`docs/sql.md`).
//!
//! # What this crate is
//!
//! A [`SqlEngine`] wraps a DataFusion `SessionContext` over an in-memory
//! [`SessionBinding`] (`GameDefs` + `World` + `PricesResult`). Hosts bind the
//! session with Rust [`SqlEngine::use_save`] (never a mutating `SELECT`), then
//! run ad-hoc SQL via [`SqlEngine::query`] / [`query`].
//!
//! Normative column/UDF contracts live in `docs/sql.md`. This crate’s rustdoc
//! is the implementer map: modules, pushdown, arg/NULL rules, and how MCP /
//! Advanced Query consume the engine.
//!
//! # Session model
//!
//! | Construction | Catalog `saves` | `active.*` / unqualified | `latest.*` | UDFs |
//! | --- | --- | --- | --- | --- |
//! | [`SqlEngine::bind`] | no | bound immediately | no | registered |
//! | [`SqlEngine::with_catalog`] | yes | [`SqlError::Unbound`] until [`SqlEngine::use_save`] | lazy max-`mtime` load (`install = false`) | registered on `use_save` |
//!
//! - **`use_save`** — host API only. Loads/solves via `vic3-api`, installs the
//!   process analysis session, rebinds `active.*` + unqualified names + UDFs.
//! - **Read-only `SELECT`** — [`readonly::assert_readonly`] rejects DDL/DML
//!   before planning. No `set_active_save` UDF; no path arguments in SQL.
//! - **`active.*`** — fact tables for the bound session (no `save_id` column).
//! - **Unqualified** (e.g. `states`) — same providers as `active.*` after bind /
//!   `use_save`; [`providers::UnboundFactProvider`] before. Player-scoped short
//!   names return only the played country / its states; full save is `world_*`.
//! - **`latest.*`** — same schemas, pinned to catalog max-`mtime` at **scan**
//!   time; must not mutate the active session (`mtime` ≠ in-game date).
//!
//! # Module map
//!
//! | Module | Role |
//! | --- | --- |
//! | [`session`] | [`SqlEngine`]: `bind` / `with_catalog` / `use_save` / `query` |
//! | `host` | Catalog + active meta + `latest.*` cache; [`UseSaveRequest`] |
//! | [`binding`] | Snapshot held by providers/UDFs; shortage helper |
//! | [`providers`] | Fact table providers, `saves`, `latest` / unbound wrappers |
//! | [`providers::pushdown`] | Exact equality vs Exact range classification |
//! | [`scope`] | Player vs `world_*` row filtering (`player_tag`) |
//! | [`schema`] | Arrow schemas for facts + TVFs (`List<Struct{…}>` IO) |
//! | [`udfs`] | Diagnostics scalars/TVFs + `plan` / `gaps` |
//! | [`readonly`] | Parse-time SELECT/EXPLAIN gate |
//! | `format` | `columns`/`rows`/`row_count` JSON (+ CSV) for UI / agents |
//! | [`error`] | [`SqlError`] / path-free [`SaveCandidate`] |
//! | `filter` / `exec` | Predicate decode; in-memory execution plans |
//!
//! # Providers and pushdown
//!
//! Fact providers materialize Arrow batches from the binding. Filter pushdown
//! is **Exact** when we can guarantee every returned row satisfies the
//! predicate (DataFusion will not re-filter):
//!
//! | Backing | Equality (`=`) | Range / `BETWEEN` |
//! | --- | --- | --- |
//! | Hash / `index_of` / bind-time name map | Exact | Unsupported |
//! | `BTreeMap` (defs `type_id`, `pm`) | Exact | Exact (`range_str`) |
//!
//! Per-table `PUSH` constants live next to each provider. List/`array_has`
//! filters are not pushed (DF filters after scan).
//!
//! **IO columns:** `buildings.input_goods` / `output_goods` and
//! `production_methods.inputs` / `outputs` are
//! `List<Struct{good, good_name, qty}>` (script id, not bare `GoodId`).
//! Explode with SELECT-list `unnest(unnest(col))` — lateral `FROM … UNNEST`
//! is not supported in this DataFusion version.
//!
//! # UDFs / TVFs
//!
//! Registered by [`udfs::register`] on `bind` / `use_save` over the active
//! binding:
//!
//! | Name | Kind | Args (plan-time literals) | NULL |
//! | --- | --- | --- | --- |
//! | `good_price(good)` | scalar | Utf8 | arg or missing id → NULL |
//! | `army_power()` | scalar | none | no player tag → NULL |
//! | `player_tag()` | scalar | none | no player tag → NULL |
//! | `is_underemployed(state_id)` | scalar | Int64 | NULL arg → NULL; else Underemployed alert |
//! | `alerts([scope])` | TVF | optional `'all'` | zero-arg → player-scoped; `'all'` → full save |
//! | `suggest_mitigations([scope])` | TVF | optional `'player'` / `'all'` | heuristic alert mitigations as rows (not sized-to-fix) |
//! | `shortage_analysis(good)` | TVF | Utf8 or NULL | NULL = all scarce-good alerts |
//! | `building_staffing(state_id)` | TVF | non-null int | NULL rejected |
//! | `plan(goal [, max_days [, label]])` | TVF | non-null strings/int | NULL rejected; `max_days` default 3650; `label` ignored in rows |
//! | `gaps(goal)` | TVF | non-null string | NULL rejected |
//!
//! TVF args are evaluated at UDTF `call` time ([`udfs::args`]); expression
//! args are plan errors.
//!
//! # Consumers
//!
//! - **MCP `query`** (`vic3-mcp`): same [`SqlEngine`] behind a mutex; tool
//!   formats batches to the `columns`/`rows`/`row_count` shape (currently a
//!   parallel formatter with the same contract as [`batches_to_json`]).
//!   `use_save` / `refresh_catalog` are sibling tools. `vic3://schema` serves
//!   [`schema_catalog_json`].
//! - **Advanced Query UI** (Tauri): `sql_query` → [`query`] + [`batches_to_json`];
//!   Saves tab / MCP share one engine so `use_save` bindings match. In-app docs
//!   embed `docs/sql.md`.
//!
//! # Re-exports
//!
//! Public surface: [`SqlEngine`], [`SessionBinding`], [`SqlError`],
//! [`UseSaveRequest`] / [`UseSaveResult`], [`EngineLoadOpts`],
//! [`ActiveSessionInfo`], [`FactTable`] / [`FACT_TABLES`],
//! [`batches_to_json`] / [`batches_to_csv`], [`schema_catalog_json`], [`query`].

pub mod binding;
pub mod error;
mod exec;
mod filter;
pub mod format;
mod host;
pub mod providers;
pub mod readonly;
pub mod schema;
pub mod scope;
pub mod session;
pub mod udfs;

pub use binding::SessionBinding;
pub use error::{SaveCandidate, SqlError};
pub use format::{batches_to_csv, batches_to_json, FormatError};
pub use host::{EngineLoadOpts, UseSaveRequest, UseSaveResult};
pub use providers::{FactTable, FACT_TABLES};
pub use session::{ActiveSessionInfo, SqlEngine};

use datafusion::arrow::array::RecordBatch;

/// Static schema registry for MCP `vic3://schema` / agent discovery.
///
/// Mirrors fact tables, diagnostics + planning TVFs, and scalars from
/// `docs/sql.md`. Keep in sync with [`schema`] and [`providers::FACT_TABLES`].
pub fn schema_catalog_json() -> serde_json::Value {
    use providers::FACT_TABLES;
    let tables: Vec<serde_json::Value> = FACT_TABLES
        .iter()
        .flat_map(|t| {
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
            let mut entries = vec![serde_json::json!({
                "name": t.name(),
                "schemas": ["active", "latest", "(unqualified after use_save)"],
                "scope": if t.is_player_scoped() { "player" } else { "session" },
                "columns": fields,
            })];
            if let Some(world_name) = t.world_name() {
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
                entries.push(serde_json::json!({
                    "name": world_name,
                    "schemas": ["active", "latest", "(unqualified after use_save)"],
                    "scope": "world",
                    "columns": fields,
                }));
            }
            entries
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

    let cols = |schema: datafusion::arrow::datatypes::SchemaRef| -> Vec<String> {
        schema
            .fields()
            .iter()
            .map(|f| f.name().to_string())
            .collect()
    };

    serde_json::json!({
        "catalog_table": {
            "name": "saves",
            "columns": saves_fields,
        },
        "fact_tables": tables,
        "scalars": [
            {
                "name": "good_price",
                "signature": "good_price(good TEXT) → FLOAT",
                "null": "NULL arg or unknown good → NULL",
            },
            {
                "name": "army_power",
                "signature": "army_power() → FLOAT",
                "null": "no resolvable player_tag → NULL",
            },
            {
                "name": "player_tag",
                "signature": "player_tag() → TEXT",
                "null": "no player_tag on bound world → NULL",
            },
            {
                "name": "is_underemployed",
                "signature": "is_underemployed(state_id BIGINT) → BOOLEAN",
                "null": "NULL arg → NULL; else true iff AlertKind::Underemployed for that state",
            },
        ],
        "tvfs": [
            {
                "name": "alerts",
                "signature": "alerts([scope])",
                "scope": "zero-arg = player-owned state_id or NULL; alerts('all') = full save",
                "columns": cols(schema::alerts_schema()),
            },
            {
                "name": "suggest_mitigations",
                "signature": "suggest_mitigations([scope])",
                "scope": "zero-arg / 'player' = player-scoped; 'all' = full save",
                "limitation": "exposes existing heuristic mitigations (often +1 levels); does not size actions to clear the problem",
                "columns": cols(schema::suggest_mitigations_schema()),
            },
            {
                "name": "shortage_analysis",
                "signature": "shortage_analysis(good TEXT)",
                "null_arg": "NULL = all electricity/transportation/goods shortage alerts",
                "columns": cols(schema::shortage_analysis_schema()),
            },
            {
                "name": "building_staffing",
                "signature": "building_staffing(state_id BIGINT)",
                "columns": cols(schema::building_staffing_schema()),
            },
            {
                "name": "plan",
                "signature": "plan(goal [, max_days [, label]])",
                "columns": cols(schema::plan_schema()),
            },
            {
                "name": "gaps",
                "signature": "gaps(goal)",
                "columns": cols(schema::gaps_schema()),
            },
        ],
        "notes": [
            "Session binding is host use_save — never SELECT set_active_save(...).",
            "SQL is read-only: SELECT / WITH…SELECT / EXPLAIN only.",
            "Stubs are filename basenames; no filesystem path arguments.",
            "Unqualified fact names require use_save/bind; they do not fall back to latest.*.",
            "Player-scoped short names (states, countries, …) filter to World.player_tag; use world_* for the full save.",
            "TVF arguments must be plan-time literals.",
            "IO columns are List<Struct{good, good_name, qty}>; use unnest(unnest(col)).",
        ],
    })
}

/// Run `sql` against an engine on a current-thread Tokio runtime.
///
/// Prefer [`SqlEngine::query`] when already inside an async context.
///
/// # Errors
///
/// Returns [`SqlError::ReadOnly`] for non-SELECT statements, [`SqlError::Unbound`]
/// when fact tables are queried before bind/`use_save`, or DataFusion/API errors
/// wrapped as [`SqlError`].
pub fn query(engine: &SqlEngine, sql: &str) -> Result<Vec<RecordBatch>, SqlError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SqlError::internal(format!("tokio runtime: {e}")))?;
    rt.block_on(engine.query(sql))
}
