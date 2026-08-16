//! JSON Schema for wasm / UI forms. Mirrors `docs/json-schema.md`.
//!
use schemars::{schema_for, JsonSchema};

fn to_json<T: JsonSchema>() -> String {
    serde_json::to_string(&schema_for!(T)).expect("JSON Schema serializes")
}

/// JSON Schema for [`vic3_prices::WhatIfOpts`].
pub fn what_if_schema_json() -> String {
    to_json::<vic3_prices::WhatIfOpts>()
}

/// JSON Schema for [`vic3_prices::PricesResult`] (includes residual / limitations).
pub fn prices_schema_json() -> String {
    to_json::<vic3_prices::PricesResult>()
}
