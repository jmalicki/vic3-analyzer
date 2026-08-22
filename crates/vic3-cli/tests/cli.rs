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
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "vic3-cli-archive-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
    let goods = value
        .get("goods")
        .and_then(Value::as_array)
        .expect("goods array");
    assert!(
        value["inputs"]["goods_with_orders"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "fixture should place market orders: {value}"
    );
    assert!(
        goods.iter().any(|good| {
            good["price"]
                .as_f64()
                .zip(good["base"].as_f64())
                .is_some_and(|(price, base)| price != base)
        }),
        "fixture orders should move at least one price from base: {value}"
    );
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
fn defs_export_writes_browser_blob_from_game_tree() {
    let root = temp_archive();
    let out = root.join("defs.postcard");
    bin()
        .args([
            "defs",
            "export",
            "--game",
            defs_fixture().to_str().expect("utf8 defs path"),
            "--out",
            out.to_str().expect("utf8 output path"),
        ])
        .assert()
        .success();

    let blob = std::fs::read(&out).expect("exported defs blob");
    let defs = vic3_defs::decode_blob(&blob).expect("valid postcard defs");
    assert_eq!(defs.goods.len(), 3);

    let assert = bin()
        .args([
            "prices",
            "--save",
            save_fixture().to_str().expect("utf8 save path"),
            "--defs",
            out.to_str().expect("utf8 defs blob"),
            "--json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let value: Value = serde_json::from_str(&stdout).expect("PricesResult JSON");
    assert_prices_json(&value);

    std::fs::remove_dir_all(root).unwrap();
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
fn plan_research_fixture_costs_tech_prereq_days() {
    let archive = temp_archive();
    let assert = plan_cmd(&archive).assert().success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("PlanResult JSON");

    // Fixture: manufacturies(50) + shaft_mining(75) + nitroglycerin(100).
    assert_eq!(value["day_cost"], 225);
    assert_eq!(value["actions"].as_array().unwrap().len(), 6);
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
    assert_eq!(record["result"]["day_cost"], 225);
    std::fs::remove_dir_all(archive).unwrap();
}

#[test]
fn archive_diff_emits_compare_result_json() {
    let archive = temp_archive();
    let records_dir = archive.join("vic3-analyzer");
    std::fs::create_dir_all(&records_dir).unwrap();
    // Synthetic archive rows (not live plans): keep arbitrary day_cost values.
    for (id, day_cost) in [("left", 365), ("right", 480)] {
        let record = serde_json::json!({
            "id": id,
            "created_at": "2026-08-15T12:00:00Z",
            "label": null,
            "kind": "plan",
            "fingerprint": "same-save",
            "date": "1840.2.3",
            "country": "FRA",
            "filename": "campaign.v3",
            "opts": {"goal": "research(tech=nitroglycerin)"},
            "result": {
                "day_cost": day_cost,
                "actions": [],
                "limitations": [],
                "residual": 0.0
            },
            "limitations": [],
            "parent_id": null,
            "blob": null
        });
        std::fs::write(
            records_dir.join(format!("{id}.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    let output = bin()
        .env("XDG_DATA_HOME", &archive)
        .args(["archive", "diff", "left", "right"])
        .assert()
        .success();
    let comparison: Value =
        serde_json::from_slice(&output.get_output().stdout).expect("CompareResult JSON");
    assert_eq!(comparison["left"], "left");
    assert_eq!(comparison["right"], "right");
    assert_eq!(comparison["same_fingerprint"], true);
    assert_eq!(comparison["day_cost_delta"], 115);
    assert!(comparison.get("actions").is_none());
    std::fs::remove_dir_all(archive).unwrap();
}

#[test]
fn fixtures_exist() {
    assert!(Path::new(&defs_fixture()).join("common/goods").is_dir());
    assert!(save_fixture().is_file());
    assert!(barren_save_fixture().is_file());
}

#[test]
fn help_lists_alerts_mutate_optimize_and_export() {
    let assert = bin().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    for command in ["alerts", "mutate", "optimize-pms", "export-save"] {
        assert!(
            stdout.contains(command),
            "expected `{command}` in help:\n{stdout}"
        );
    }
}

#[test]
fn export_save_writes_a_different_path() {
    let root = temp_archive();
    let out = root.join("patched.v3");
    let save = save_fixture();
    assert_ne!(out, save);

    bin()
        .args([
            "export-save",
            "--save",
            save.to_str().expect("utf8 save path"),
            "--delta-json",
            r#"{"production_methods":[{"building_id":1,"methods":["pm_soil_enriching_farming"]}]}"#,
            "--out",
            out.to_str().expect("utf8 out path"),
        ])
        .assert()
        .success();

    let original = std::fs::read(&save).expect("original save");
    let patched = std::fs::read(&out).expect("patched save");
    assert_ne!(patched, original);
    let patched_text = String::from_utf8_lossy(&patched);
    assert!(
        patched_text.contains("pm_soil_enriching_farming"),
        "patched save should contain the new PM:\n{patched_text}"
    );
    let original_text = String::from_utf8_lossy(&original);
    assert!(
        !original_text.contains("pm_soil_enriching_farming"),
        "origin save must stay unchanged"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mcp_status_table_and_json() {
    let assert = bin().args(["mcp", "status"]).assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("Claude Desktop"), "{stdout}");
    assert!(stdout.contains("LM Studio"), "{stdout}");

    let assert_json = bin().args(["mcp", "status", "--json"]).assert().success();
    let json_stdout = String::from_utf8_lossy(&assert_json.get_output().stdout);
    let value: Value = serde_json::from_str(&json_stdout).expect("valid JSON array");
    let arr = value.as_array().expect("array of status objects");
    assert!(!arr.is_empty());
    assert!(arr.iter().any(|item| item["id"] == "claude-desktop"));
    assert!(arr.iter().any(|item| item["id"] == "lm-studio"));
}

#[test]
fn mcp_show_config_snippets() {
    let assert_claude = bin()
        .args(["mcp", "show-config", "--client", "claude-desktop"])
        .assert()
        .success();
    let stdout_claude = String::from_utf8_lossy(&assert_claude.get_output().stdout);
    assert!(stdout_claude.contains("mcpServers"), "{stdout_claude}");
    assert!(stdout_claude.contains("vic3-analyzer"), "{stdout_claude}");

    let assert_codex = bin()
        .args(["mcp", "show-config", "--client", "codex"])
        .assert()
        .success();
    let stdout_codex = String::from_utf8_lossy(&assert_codex.get_output().stdout);
    assert!(
        stdout_codex.contains("[mcp_servers.vic3-analyzer]"),
        "{stdout_codex}"
    );
}

#[test]
fn mcp_install_and_uninstall_dry_run() {
    let assert_install = bin()
        .args(["mcp", "install", "--client", "claude-desktop", "--dry-run"])
        .assert()
        .success();
    let stdout_install = String::from_utf8_lossy(&assert_install.get_output().stdout);
    assert!(stdout_install.contains("[DRY-RUN]"), "{stdout_install}");
    assert!(stdout_install.contains("vic3-analyzer"), "{stdout_install}");

    let assert_uninstall = bin()
        .args([
            "mcp",
            "uninstall",
            "--client",
            "claude-desktop",
            "--dry-run",
        ])
        .assert()
        .success();
    let stdout_uninstall = String::from_utf8_lossy(&assert_uninstall.get_output().stdout);
    assert!(stdout_uninstall.contains("[DRY-RUN]"), "{stdout_uninstall}");
}
