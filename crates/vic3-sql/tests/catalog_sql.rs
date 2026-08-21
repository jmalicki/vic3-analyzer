//! Catalog + `use_save` + `active.*` / `latest.*` integration tests.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use datafusion::arrow::array::{Array, BooleanArray, StringArray};
use tempfile::TempDir;
use vic3_catalog::{scan_roots, SaveLocation, SaveRoot};
use vic3_defs::{encode_blob, load_from_path};
use vic3_sql::{EngineLoadOpts, SqlEngine, SqlError, UseSaveRequest};

fn defs_blob_path(tmp: &TempDir) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures");
    let defs = load_from_path(&root).expect("defs fixture");
    let blob = encode_blob(&defs).expect("encode defs");
    let path = tmp.path().join("defs.blob");
    std::fs::write(&path, blob).expect("write defs blob");
    path
}

fn save_fixture() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-load/tests/fixtures/plaintext.txt"),
    )
    .expect("plaintext save")
}

fn write_v3(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join(name), bytes).expect("write save");
}

async fn catalog_engine(tmp: &TempDir, roots: Vec<SaveRoot>) -> SqlEngine {
    let catalog = scan_roots(&roots).expect("scan");
    let load = EngineLoadOpts::new(defs_blob_path(tmp));
    SqlEngine::with_catalog(catalog, load)
        .await
        .expect("with_catalog")
}

#[tokio::test]
async fn stub_use_save_then_active_queries() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join("local");
    std::fs::create_dir_all(&local).unwrap();
    write_v3(&local, "autosave.v3", &save_fixture());

    let eng = catalog_engine(
        &tmp,
        vec![SaveRoot {
            path: local,
            location: SaveLocation::Local,
        }],
    )
    .await;

    let saves = eng
        .query("SELECT name, kind, loaded FROM saves ORDER BY name")
        .await
        .expect("saves");
    let names = saves[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(names.value(0), "autosave");
    let loaded = saves[0]
        .column(2)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(!loaded.value(0));

    let unbound = eng
        .query("SELECT tag FROM countries")
        .await
        .expect_err("unbound");
    assert!(
        matches!(unbound, SqlError::DataFusion(_))
            || unbound.to_string().contains("no active session"),
        "{unbound}"
    );

    let result = eng
        .use_save(UseSaveRequest {
            name: Some("autosave".into()),
            ..Default::default()
        })
        .await
        .expect("use_save");
    assert_eq!(result.name, "autosave");
    assert!(result.loaded);
    assert_eq!(result.country.as_deref(), Some("GER"));

    let countries = eng
        .query("SELECT tag FROM active.countries WHERE tag = 'GER'")
        .await
        .expect("active.countries");
    let tags = countries[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(tags.value(0), "GER");

    let unqualified = eng
        .query("SELECT tag FROM countries WHERE tag = 'GER'")
        .await
        .expect("unqualified countries");
    assert_eq!(unqualified[0].num_rows(), 1);

    let constructions = eng
        .query("SELECT queue, building FROM constructions")
        .await
        .expect("constructions fact table");
    assert_eq!(
        constructions[0].schema().field(0).name(),
        "queue",
        "constructions registered after use_save"
    );

    let saves_after = eng
        .query("SELECT loaded, in_game_date, country FROM saves WHERE name = 'autosave'")
        .await
        .expect("loaded flag");
    let loaded = saves_after[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(loaded.value(0));
    let dates = saves_after[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(
        !dates.is_null(0),
        "in_game_date should be patched after use_save"
    );
    assert!(!dates.value(0).is_empty());
    let countries_col = saves_after[0]
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(countries_col.value(0), "GER");
}

#[tokio::test]
async fn ambiguous_stub_errors_with_candidates() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join("local");
    let cloud = tmp.path().join("cloud");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&cloud).unwrap();
    let bytes = save_fixture();
    write_v3(&local, "autosave.v3", &bytes);
    write_v3(&cloud, "autosave.v3", &bytes);

    let eng = catalog_engine(
        &tmp,
        vec![
            SaveRoot {
                path: local,
                location: SaveLocation::Local,
            },
            SaveRoot {
                path: cloud,
                location: SaveLocation::SteamCloud,
            },
        ],
    )
    .await;

    let err = eng
        .use_save(UseSaveRequest {
            name: Some("autosave".into()),
            ..Default::default()
        })
        .await
        .expect_err("ambiguous");
    match err {
        SqlError::Ambiguous { stub, candidates } => {
            assert_eq!(stub, "autosave");
            assert_eq!(candidates.len(), 2);
            let locs: Vec<_> = candidates.iter().map(|c| c.location).collect();
            assert!(locs.contains(&SaveLocation::Local));
            assert!(locs.contains(&SaveLocation::SteamCloud));
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }

    let ok = eng
        .use_save(UseSaveRequest {
            name: Some("autosave".into()),
            location: Some(SaveLocation::SteamCloud),
            ..Default::default()
        })
        .await
        .expect("disambiguated");
    assert_eq!(ok.name, "autosave");
}

#[tokio::test]
async fn selectors_and_latest_views_do_not_mutate_active() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("saves");
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = save_fixture();
    write_v3(&dir, "old_named.v3", &bytes);
    thread::sleep(Duration::from_millis(30));
    write_v3(&dir, "autosave.v3", &bytes);
    thread::sleep(Duration::from_millis(30));
    write_v3(&dir, "new_named.v3", &bytes);

    let eng = catalog_engine(
        &tmp,
        vec![SaveRoot {
            path: dir,
            location: SaveLocation::Local,
        }],
    )
    .await;

    let latest_named = eng
        .use_save(UseSaveRequest {
            selector: Some("latest_named".into()),
            ..Default::default()
        })
        .await
        .expect("latest_named");
    assert_eq!(latest_named.name, "new_named");

    // Point active at autosave, then query latest.* (mtime winner = new_named)
    // without changing the active session.
    eng.use_save(UseSaveRequest {
        name: Some("autosave".into()),
        ..Default::default()
    })
    .await
    .expect("bind autosave");

    let active_name = eng
        .query("SELECT name FROM saves WHERE loaded = true")
        .await
        .expect("active loaded");
    let active = active_name[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(active.value(0), "autosave");

    let latest_countries = eng
        .query("SELECT tag FROM latest.countries WHERE tag = 'GER'")
        .await
        .expect("latest.countries");
    assert_eq!(latest_countries[0].num_rows(), 1);

    let still_autosave = eng
        .query("SELECT name FROM saves WHERE loaded = true")
        .await
        .expect("still autosave");
    let name = still_autosave[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(name.value(0), "autosave");

    let by_selector = eng
        .use_save(UseSaveRequest {
            selector: Some("latest_autosave".into()),
            ..Default::default()
        })
        .await
        .expect("latest_autosave");
    assert_eq!(by_selector.name, "autosave");

    let latest = eng
        .use_save(UseSaveRequest {
            selector: Some("latest".into()),
            ..Default::default()
        })
        .await
        .expect("latest");
    assert_eq!(latest.name, "new_named");
}

#[tokio::test]
async fn rejects_mutating_select_style_statements() {
    let tmp = TempDir::new().unwrap();
    let eng = catalog_engine(&tmp, vec![]).await;
    let err = eng
        .query("SELECT set_active_save('autosave')")
        .await
        .expect_err("no mutating udf");
    // Either parse/plan failure or read-only — must not succeed.
    let _ = err;
    let ddl = eng
        .query("UPDATE saves SET loaded = true")
        .await
        .expect_err("dml");
    assert!(matches!(ddl, SqlError::ReadOnly(_)));
}
