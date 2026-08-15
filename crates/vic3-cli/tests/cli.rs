//! assert_cmd tests against defs + plaintext save fixtures.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn bin() -> Command {
    Command::cargo_bin("vic3-cli").expect("vic3-cli bin")
}

fn defs_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures")
}

fn save_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-load/tests/fixtures/plaintext.txt")
}

fn prices_cmd() -> Command {
    let mut cmd = bin();
    cmd.args([
        "prices",
        "--save",
        save_fixture().to_str().expect("utf8 save path"),
        "--game",
        defs_fixture().to_str().expect("utf8 defs path"),
    ]);
    cmd
}

fn what_if_cmd() -> Command {
    let mut cmd = bin();
    cmd.args([
        "what-if",
        "--save",
        save_fixture().to_str().expect("utf8 save path"),
        "--game",
        defs_fixture().to_str().expect("utf8 defs path"),
        "--building",
        "building_rye_farm",
        "--extra-levels",
        "5",
    ]);
    cmd
}

fn assert_prices_json(value: &Value) {
    assert!(
        value.get("residual").and_then(Value::as_f64).is_some(),
        "I5: residual must be present, got {value}"
    );
    let limitations = value
        .get("limitations")
        .and_then(Value::as_array)
        .expect("limitations array");
    assert!(
        !limitations.is_empty(),
        "limitations must be present and non-empty"
    );
    assert!(value.get("goods").and_then(Value::as_array).is_some());
    assert!(value.get("status").and_then(Value::as_str).is_some());
}

#[test]
fn prices_json_has_residual_and_limitations() {
    let assert = prices_cmd().arg("--json").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let value: Value = serde_json::from_str(&stdout).expect("PricesResult JSON");
    assert_prices_json(&value);
}

#[test]
fn prices_text_table_and_limitations_warning() {
    let assert = prices_cmd().assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stdout.contains("residual"),
        "table should report residual:\n{stdout}"
    );
    assert!(
        stderr.contains("warning:"),
        "one-line limitations warning on stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("Employment, wages, and trade volumes are frozen"),
        "warning should include a LIMITATIONS string:\n{stderr}"
    );
    assert_eq!(
        stderr.trim().lines().count(),
        1,
        "limitations warning must be one line:\n{stderr}"
    );
}

#[test]
fn what_if_json_has_residual_and_limitations() {
    let assert = what_if_cmd().arg("--json").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let value: Value = serde_json::from_str(&stdout).expect("PricesResult JSON");
    assert_prices_json(&value);
}

#[test]
fn fixtures_exist() {
    assert!(Path::new(&defs_fixture()).join("common/goods").is_dir());
    assert!(save_fixture().is_file());
}
