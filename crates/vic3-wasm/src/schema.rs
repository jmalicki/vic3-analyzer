//! JSON Schema for wasm / UI forms. Mirrors `docs/json-schema.md`.
//!
//! Local schemars types so this crate does not need `JsonSchema` on
//! `vic3-prices` (P5a may add the same derives there).

use schemars::{schema_for, JsonSchema};
use serde::Serialize;

/// Extra building levels applied before a re-solve.
#[derive(JsonSchema, Serialize)]
struct WhatIfOpts {
    /// Building type id.
    building: String,
    /// Non-negative extra levels added to matching buildings.
    extra_levels: u32,
}

/// Why the solver stopped.
#[allow(dead_code)] // variants exist for schemars / serde rename only
#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum SolveStatus {
    Converged,
    MaxIters,
    Failed,
}

/// One row of the goods table.
#[derive(JsonSchema, Serialize)]
struct GoodPrice {
    id: String,
    base: f64,
    price: f64,
    buy: f64,
    sell: f64,
}

/// Price-equilibrium output. `limitations` is always present.
#[derive(JsonSchema, Serialize)]
struct PricesResult {
    goods: Vec<GoodPrice>,
    /// `‖r − r_formula(orders(r))‖₂`. Always present (I5).
    residual: f64,
    status: SolveStatus,
    limitations: Vec<String>,
}

fn to_json<T: JsonSchema>() -> String {
    serde_json::to_string(&schema_for!(T)).expect("JSON Schema serializes")
}

/// JSON Schema for [`vic3_prices::WhatIfOpts`].
pub fn what_if_schema_json() -> String {
    to_json::<WhatIfOpts>()
}

/// JSON Schema for [`vic3_prices::PricesResult`] (includes residual / limitations).
pub fn prices_schema_json() -> String {
    to_json::<PricesResult>()
}
