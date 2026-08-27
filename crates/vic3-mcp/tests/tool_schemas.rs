//! Tool / prompt schema contract tests (`docs/mcp.md`).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use vic3_catalog::{AppConfig, SaveLocation, SaveRoot};
use vic3_mcp::{McpRuntime, Vic3McpServer};
use vic3_prices::{ExtraLevelsDelta, WorldDelta};
use vic3_sql::{EngineLoadOpts, SqlEngine, UseSaveRequest};

fn write_save(dir: &std::path::Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(name), b"SAV").unwrap();
}

fn defs_blob(tmp: &TempDir) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures");
    let defs = vic3_defs::load_from_path(&root).expect("defs");
    let blob = vic3_defs::encode_blob(&defs).expect("encode");
    let path = tmp.path().join("defs.postcard");
    fs::write(&path, blob).unwrap();
    path
}

fn save_fixture() -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-load/tests/fixtures/plaintext.txt"),
    )
    .expect("plaintext")
}

async fn server_with_saves(tmp: &TempDir) -> Vic3McpServer {
    let saves = tmp.path().join("saves");
    write_save(&saves, "autosave.v3");
    write_save(&saves, "Campaign.v3");
    let blob = defs_blob(tmp);

    let config = AppConfig {
        auto_detect: false,
        save_dirs: vec![saves],
        defs_blob: Some(blob),
        ..Default::default()
    };

    let runtime = McpRuntime::from_config(
        tmp.path().to_path_buf(),
        tmp.path().join("config.toml"),
        config,
    )
    .await
    .expect("runtime");
    Vic3McpServer::new(runtime)
}

#[tokio::test]
async fn tool_schemas_match_mcp_contract() {
    let tmp = TempDir::new().unwrap();
    let server = server_with_saves(&tmp).await;
    let tools = server.tool_router_ref().list_all();
    let names: BTreeSet<_> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "query",
            "use_save",
            "refresh_catalog",
            "explain",
            "campaign_brief",
            "preview_delta",
        ])
    );

    let by_name = |n: &str| tools.iter().find(|t| t.name == n).expect(n);

    let query = by_name("query");
    let qschema = query.input_schema.as_ref();
    assert!(
        qschema
            .get("properties")
            .and_then(|p| p.get("sql"))
            .is_some(),
        "query must require sql: {qschema:?}"
    );
    // format is optional
    assert!(qschema
        .get("properties")
        .and_then(|p| p.get("format"))
        .is_some());

    let use_save = by_name("use_save");
    let uschema = use_save.input_schema.as_ref();
    let props = uschema.get("properties").expect("properties");
    for key in ["name", "selector", "location", "mtime"] {
        assert!(props.get(key).is_some(), "missing {key} in {uschema:?}");
    }

    let refresh = by_name("refresh_catalog");
    assert!(
        refresh.input_schema.as_ref().get("type").is_some()
            || refresh.input_schema.as_ref().is_empty()
            || refresh.input_schema.as_ref().contains_key("properties"),
        "refresh_catalog should expose an object schema"
    );

    let brief = by_name("campaign_brief");
    assert!(
        brief.input_schema.as_ref().get("type").is_some()
            || brief.input_schema.as_ref().is_empty()
            || brief.input_schema.as_ref().contains_key("properties"),
        "campaign_brief should expose an object schema"
    );

    let explain = by_name("explain");
    assert!(explain
        .input_schema
        .as_ref()
        .get("properties")
        .and_then(|p| p.get("sql"))
        .is_some());

    let preview = by_name("preview_delta");
    let pschema = preview.input_schema.as_ref();
    let pprops = pschema.get("properties").expect("preview_delta properties");
    for key in [
        "building",
        "extra_levels",
        "building_id",
        "state_id",
        "delta",
    ] {
        assert!(pprops.get(key).is_some(), "missing {key} in {pschema:?}");
    }
    assert_eq!(
        pschema.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false)),
        "preview_delta must deny unknown fields: {pschema:?}"
    );
}

#[tokio::test]
async fn prompt_names_are_stable() {
    let tmp = TempDir::new().unwrap();
    let server = server_with_saves(&tmp).await;
    let prompts = server.prompt_router_ref().list_all();
    let names: BTreeSet<_> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "investigate_shortages",
            "compare_latest_autosave",
            "military_readiness",
            "what_is_loaded",
            "plan_research",
        ])
    );
}

#[tokio::test]
async fn query_and_use_save_tools_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let saves = tmp.path().join("saves");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("autosave.v3"), save_fixture()).unwrap();
    let blob = defs_blob(&tmp);

    let config = AppConfig {
        auto_detect: false,
        save_dirs: vec![saves.clone()],
        defs_blob: Some(blob.clone()),
        ..Default::default()
    };

    let runtime = McpRuntime::from_config(
        tmp.path().to_path_buf(),
        tmp.path().join("config.toml"),
        config,
    )
    .await
    .expect("runtime");

    let batches = runtime
        .query("SELECT name, kind FROM saves ORDER BY name")
        .await
        .expect("saves query");
    assert!(!batches.is_empty());
    assert!(batches[0].num_rows() >= 1);

    let result = runtime
        .use_save(UseSaveRequest {
            name: Some("autosave".into()),
            ..Default::default()
        })
        .await
        .expect("use_save");
    assert!(result.loaded);
    assert_eq!(result.country.as_deref(), Some("GER"));

    let brief = runtime.campaign_brief().await.expect("campaign_brief");
    for key in [
        "session",
        "player_tag",
        "top_goods",
        "hotspots",
        "alert_kinds",
    ] {
        assert!(brief.get(key).is_some(), "missing {key} in {brief}");
    }
    let session = brief.get("session").expect("session");
    for key in ["name", "kind", "in_game_date", "country"] {
        assert!(session.get(key).is_some(), "missing session.{key}");
    }
    assert_eq!(brief["player_tag"], "GER");
    assert!(brief["top_goods"].is_array());
    assert!(brief["hotspots"].is_array());
    assert!(brief["alert_kinds"].is_object());

    let countries = runtime
        .query("SELECT name FROM countries WHERE name = 'GER'")
        .await
        .expect("countries");
    assert_eq!(countries[0].num_rows(), 1);

    let count = runtime.refresh_catalog().await.expect("refresh");
    assert_eq!(count, 1);

    // EngineLoadOpts path still works standalone (shared SQL contract).
    let catalog = vic3_catalog::scan_roots(&[SaveRoot {
        path: saves,
        location: SaveLocation::Local,
    }])
    .unwrap();
    let eng = SqlEngine::with_catalog(catalog, EngineLoadOpts::new(blob))
        .await
        .unwrap();
    let _ = eng;
}

#[tokio::test]
async fn campaign_brief_requires_bound_save() {
    let tmp = TempDir::new().unwrap();
    let saves = tmp.path().join("saves");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("autosave.v3"), save_fixture()).unwrap();
    let blob = defs_blob(&tmp);

    let config = AppConfig {
        auto_detect: false,
        save_dirs: vec![saves],
        defs_blob: Some(blob),
        ..Default::default()
    };

    let runtime = McpRuntime::from_config(
        tmp.path().to_path_buf(),
        tmp.path().join("config.toml"),
        config,
    )
    .await
    .expect("runtime");

    let err = runtime.campaign_brief().await.expect_err("unbound");
    assert!(
        err.to_string().contains("no active session binding"),
        "{err}"
    );
}

#[tokio::test]
async fn preview_delta_rye_drops_wood_price() {
    let tmp = TempDir::new().unwrap();
    let saves = tmp.path().join("saves");
    fs::create_dir_all(&saves).unwrap();
    fs::write(saves.join("autosave.v3"), save_fixture()).unwrap();
    let blob = defs_blob(&tmp);

    let config = AppConfig {
        auto_detect: false,
        save_dirs: vec![saves],
        defs_blob: Some(blob),
        ..Default::default()
    };

    let runtime = McpRuntime::from_config(
        tmp.path().to_path_buf(),
        tmp.path().join("config.toml"),
        config,
    )
    .await
    .expect("runtime");

    let unbound = runtime.preview_delta(&WorldDelta::default()).await;
    assert!(
        unbound.unwrap_err().contains("no active save"),
        "unbound session must error"
    );

    runtime
        .use_save(UseSaveRequest {
            name: Some("autosave".into()),
            ..Default::default()
        })
        .await
        .expect("use_save");

    let delta = WorldDelta {
        extra_levels: vec![ExtraLevelsDelta {
            building: Some("building_rye_farm".into()),
            building_id: None,
            extra_levels: 1,
        }],
        ..Default::default()
    };
    let body = runtime.preview_delta(&delta).await.expect("preview_delta");

    assert_eq!(body["status"], "converged");
    assert!(body["residual"].as_f64().is_some());
    assert!(body.get("limitations").and_then(|v| v.as_array()).is_some());
    assert!(body.get("applied").is_some());
    // Compact: no full PricesResult dump fields.
    assert!(body.get("buildings").is_none());
    assert!(body.get("state_pops").is_none());
    assert!(body.get("relative").is_none());

    let goods = body["goods"].as_array().expect("goods");
    let wood = goods
        .iter()
        .find(|g| g["name"] == "wood")
        .expect("wood row among changed goods");
    let before = wood["price_before"].as_f64().unwrap();
    let after = wood["price_after"].as_f64().unwrap();
    assert!(
        after < before,
        "rye +1 (forestry PM) should drop wood price: {before} → {after}"
    );
    assert!(wood["d_price"].as_f64().unwrap() < 0.0);
}
