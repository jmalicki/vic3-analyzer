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

fn barren_save_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/barren.txt")
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

fn gaps_cmd() -> Command {
    let mut cmd = bin();
    cmd.args([
        "gaps",
        "--save",
        barren_save_fixture().to_str().expect("utf8 save path"),
        "--game",
        defs_fixture().to_str().expect("utf8 defs path"),
        "--goal",
        "declare-war(tag=FRA, wargoal=conquer_state, state=alsace)",
    ]);
    cmd
}

fn plan_cmd(archive_root: &Path) -> Command {
    let mut cmd = bin();
    cmd.env("XDG_DATA_HOME", archive_root).args([
        "plan",
        "--save",
        barren_save_fixture().to_str().expect("utf8 save path"),
        "--game",
        defs_fixture().to_str().expect("utf8 defs path"),
        "--goal",
        "research(tech=nitroglycerin)",
        "--label",
        "rush",
        "--json",
    ]);
    cmd
}

fn temp_archive() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "vic3-cli-archive-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
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
fn gaps_json_has_declare_war_atoms_and_limitations() {
    let assert = gaps_cmd().arg("--json").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let value: Value = serde_json::from_str(&stdout).expect("GapsResult JSON");

    assert_eq!(value.get("satisfied"), Some(&Value::Bool(false)));
    let gaps = value
        .get("gaps")
        .and_then(Value::as_array)
        .expect("gaps array");
    assert_eq!(gaps.len(), 4, "expected all declare-war gaps: {value}");
    assert!(
        gaps.iter().any(|gap| gap.get("InterestIn").is_some()),
        "interest gap: {gaps:?}"
    );
    assert!(
        gaps.iter().any(|gap| gap.get("ArmyPower").is_some()),
        "army gap: {gaps:?}"
    );
    assert!(
        gaps.iter().any(|gap| gap.get("GoodPrice").is_some()),
        "munitions-price gap: {gaps:?}"
    );
    assert!(
        gaps.iter().any(|gap| gap == "Solvent"),
        "solvent gap: {gaps:?}"
    );
    assert!(
        value
            .get("limitations")
            .and_then(Value::as_array)
            .is_some_and(|limitations| !limitations.is_empty()),
        "price limitations: {value}"
    );
}

#[test]
fn plan_research_fixture_costs_default_research_days() {
    let archive = temp_archive();
    let assert = plan_cmd(&archive).assert().success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("PlanResult JSON");

    assert_eq!(value["day_cost"], 365);
    assert_eq!(value["actions"].as_array().unwrap().len(), 2);
    assert!(value["actions"][0]["action"]["QueueTech"].is_object());
    assert!(value["actions"][1]["action"]["WaitForEvent"].is_object());
    assert!(value["residual"].is_number());
    assert!(!value["limitations"].as_array().unwrap().is_empty());
    std::fs::remove_dir_all(archive).unwrap();
}

#[test]
fn archive_list_and_show_persist_plan_record() {
    let archive = temp_archive();
    plan_cmd(&archive).assert().success();

    let list = bin()
        .env("XDG_DATA_HOME", &archive)
        .args(["archive", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&list.get_output().stdout);
    assert!(stdout.contains("\tplan\trush\t"), "{stdout}");
    let id = stdout.split('\t').next().expect("record id").trim();

    let show = bin()
        .env("XDG_DATA_HOME", &archive)
        .args(["archive", "show", id])
        .assert()
        .success();
    let record: Value =
        serde_json::from_slice(&show.get_output().stdout).expect("AnalysisRecord JSON");
    assert_eq!(record["id"], id);
    assert_eq!(record["kind"], "plan");
    assert_eq!(record["label"], "rush");
    assert_eq!(record["result"]["day_cost"], 365);
    std::fs::remove_dir_all(archive).unwrap();
}

#[test]
fn fixtures_exist() {
    assert!(Path::new(&defs_fixture()).join("common/goods").is_dir());
    assert!(save_fixture().is_file());
    assert!(barren_save_fixture().is_file());
}
