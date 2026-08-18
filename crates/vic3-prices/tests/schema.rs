//! Snapshot `schema_for!` against checked-in JSON Schema (docs/json-schema.md).

use std::fs;
use std::path::PathBuf;

use schemars::schema_for;
use serde_json::Value;
use vic3_prices::{PricesResult, SolveOpts, WhatIfOpts};

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema")
}

fn write_schema(name: &str, value: &Value) {
    let path = schema_dir().join(name);
    fs::create_dir_all(schema_dir()).expect("schema dir");
    let mut text = serde_json::to_string_pretty(value).expect("pretty schema");
    text.push('\n');
    fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// `VIC3_WRITE_SCHEMA=1 cargo test -p vic3-prices --test schema dump_schemas` regenerates snapshots.
#[test]
fn dump_schemas() {
    if std::env::var("VIC3_WRITE_SCHEMA").ok().as_deref() != Some("1") {
        return;
    }
    write_schema(
        "prices.json",
        &serde_json::to_value(schema_for!(PricesResult)).unwrap(),
    );
    write_schema(
        "what-if.json",
        &serde_json::to_value(schema_for!(WhatIfOpts)).unwrap(),
    );
}

fn read_schema(name: &str) -> Value {
    let path = schema_dir().join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing {} ({e}); regenerate from schema_for!",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn required(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn properties(schema: &Value) -> serde_json::Map<String, Value> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn prices_schema_matches_checked_in() {
    let actual = serde_json::to_value(schema_for!(PricesResult)).expect("serialize schema");
    let expected = read_schema("prices.json");
    assert_eq!(
        actual, expected,
        "schema/prices.json drifted from schema_for!(PricesResult)"
    );
}

#[test]
fn what_if_schema_matches_checked_in() {
    let actual = serde_json::to_value(schema_for!(WhatIfOpts)).expect("serialize schema");
    let expected = read_schema("what-if.json");
    assert_eq!(
        actual, expected,
        "schema/what-if.json drifted from schema_for!(WhatIfOpts)"
    );
}

#[test]
fn prices_schema_required_fields_match_json_schema_md() {
    let schema = serde_json::to_value(schema_for!(PricesResult)).unwrap();
    let req = required(&schema);
    for field in ["goods", "residual", "status", "limitations"] {
        assert!(
            req.iter().any(|f| f == field),
            "{field} must be required in PricesResult schema, got {req:?}"
        );
    }
    let goods = &properties(&schema)["goods"];
    assert_eq!(goods["type"], "array");
}

#[test]
fn what_if_schema_required_fields_match_json_schema_md() {
    let schema = serde_json::to_value(schema_for!(WhatIfOpts)).unwrap();
    let req = required(&schema);
    for field in ["building", "extra_levels"] {
        assert!(
            req.iter().any(|f| f == field),
            "{field} must be required in WhatIfOpts schema, got {req:?}"
        );
    }
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn solve_opts_schema_matches_json_schema_md_in_spirit() {
    let schema = serde_json::to_value(schema_for!(SolveOpts)).unwrap();
    let props = properties(&schema);
    assert!(props.contains_key("residual_eps"));
    assert!(props.contains_key("max_iters"));
    assert!(props.contains_key("warm_rel"));
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    let req = required(&schema);
    assert!(
        !req.iter()
            .any(|f| f == "residual_eps" || f == "max_iters" || f == "warm_rel"),
        "SolveOpts fields have defaults and should not be required, got {req:?}"
    );
}
