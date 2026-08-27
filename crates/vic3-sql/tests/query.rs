//! Fixture-backed SQL smoke tests (`docs/sql.md`).

use std::path::PathBuf;
use std::sync::Arc;

use datafusion::arrow::array::{Array, BooleanArray, Float64Array, StringArray, UInt32Array};
use vic3_defs::{decode_blob, encode_blob, load_from_path};
use vic3_load::{empty_tokens, load_slice};
use vic3_prices::{solve, SolveOpts, World};
use vic3_sql::{query, SqlEngine, SqlError};

fn defs_blob() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures");
    let defs = load_from_path(&root).expect("defs fixture");
    encode_blob(&defs).expect("encode defs")
}

fn save_bytes() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-load/tests/fixtures/plaintext.txt"),
    )
    .expect("plaintext save")
}

async fn engine() -> SqlEngine {
    let defs = decode_blob(&defs_blob()).expect("decode defs");
    let save = load_slice(&save_bytes(), empty_tokens()).expect("load save");
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, SolveOpts::default());
    SqlEngine::bind(defs, world, prices).await.expect("bind")
}

#[tokio::test]
async fn selects_states_and_countries() {
    let eng = engine().await;
    let batches = eng
        .query("SELECT state_id, owner_tag FROM states ORDER BY state_id")
        .await
        .expect("states");
    assert!(!batches.is_empty());
    let ids = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .expect("state_id");
    assert!(!ids.is_empty());
    let owners = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("owner_tag");
    for i in 0..owners.len() {
        if !owners.is_null(i) {
            assert_eq!(owners.value(i), "GER", "short states is player-scoped");
        }
    }

    let countries = eng
        .query("SELECT tag FROM countries")
        .await
        .expect("countries");
    let tags = countries[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("tag");
    assert_eq!(countries[0].num_rows(), 1);
    assert_eq!(tags.value(0), "GER");

    // world_* is registered even when the plaintext fixture is single-country.
    let world = eng
        .query("SELECT tag FROM world_countries ORDER BY tag")
        .await
        .expect("world_countries");
    assert!(world[0].num_rows() >= 1);
}

#[tokio::test]
async fn world_tables_include_foreign_when_present() {
    use vic3_prices::{WorldCountry, WorldState};

    let defs = decode_blob(&defs_blob()).expect("decode defs");
    let save = load_slice(&save_bytes(), empty_tokens()).expect("load save");
    let mut world = World::from_save(&save, &defs);
    assert_eq!(world.player_tag.as_deref(), Some("GER"));
    let ger_id = world
        .countries
        .iter()
        .find(|c| c.tag == "GER")
        .map(|c| c.id)
        .expect("GER");
    let fra_id = ger_id.saturating_add(100);
    world.countries.push(WorldCountry {
        id: fra_id,
        tag: "FRA".into(),
        laws: vec![],
        overlord: None,
        subject_type: None,
        states: vec![999],
        treasury: 0.0,
        weekly_balance: None,
        debt_principal: None,
        credit_limit: None,
        credit_headroom: None,
        solvent: true,
        techs: vec![],
        queued_tech: None,
        queued_building: None,
        army_power_projection: None,
        navy_power_projection: None,
        interest_states: vec![],
        interest_regions: vec![],
        infamy: None,
    });
    world.states.push(WorldState {
        id: 999,
        country: Some(fra_id),
        ..WorldState::default()
    });
    let prices = solve(&world, &defs, SolveOpts::default());
    assert!(
        prices.countries.iter().any(|c| c.tag == "FRA"),
        "solve should emit FRA"
    );
    assert!(
        prices.states.iter().any(|s| s.id == 999),
        "solve should emit state 999"
    );
    let eng = SqlEngine::bind(defs, world, prices).await.expect("bind");

    let player_n: i64 = eng
        .query("SELECT COUNT(*) FROM countries")
        .await
        .expect("player countries")[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    let world_n: i64 = eng
        .query("SELECT COUNT(*) FROM world_countries")
        .await
        .expect("world countries")[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(player_n, 1);
    assert!(
        world_n > player_n,
        "world_countries {world_n} > countries {player_n}"
    );

    let player_states: i64 = eng.query("SELECT COUNT(*) FROM states").await.unwrap()[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    let world_states: i64 = eng
        .query("SELECT COUNT(*) FROM world_states")
        .await
        .unwrap()[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert!(
        world_states > player_states,
        "world_states {world_states} > states {player_states}"
    );
}

#[tokio::test]
async fn goods_shortage_and_equality_filter() {
    let eng = engine().await;
    let batches = eng
        .query("SELECT name, buy, sell, shortage FROM goods WHERE name = 'grain'")
        .await
        .expect("goods");
    assert_eq!(batches[0].num_rows(), 1);
    let shortage = batches[0]
        .column(3)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("shortage");
    let buy = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .value(0);
    let sell = batches[0]
        .column(2)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .value(0);
    assert_eq!(shortage.value(0), (buy - sell).max(0.0));
}

#[tokio::test]
async fn buildings_expose_list_io_columns() {
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT building_id, type_id, production_methods, input_goods, output_goods \
             FROM buildings",
        )
        .await
        .expect("buildings");
    assert!(batches[0].num_rows() >= 1);
    // List columns are present (schema contract).
    assert_eq!(batches[0].schema().field(2).name(), "production_methods");
    assert!(matches!(
        batches[0].schema().field(2).data_type(),
        datafusion::arrow::datatypes::DataType::List(_)
    ));
    assert!(matches!(
        batches[0].schema().field(3).data_type(),
        datafusion::arrow::datatypes::DataType::List(_)
    ));
}

#[tokio::test]
async fn building_types_and_production_methods_from_defs() {
    let eng = engine().await;
    let types = eng
        .query("SELECT type_id FROM building_types WHERE type_id = 'building_rye_farm'")
        .await
        .expect("building_types");
    assert_eq!(types[0].num_rows(), 1);

    let pms = eng
        .query("SELECT pm FROM production_methods ORDER BY pm LIMIT 5")
        .await
        .expect("pms");
    assert!(pms[0].num_rows() >= 1);
}

#[tokio::test]
async fn join_example_from_docs() {
    let eng = engine().await;
    // Exact docs example shape (`docs/sql.md`); fixture may have zero shortages.
    let batches = eng
        .query(
            "SELECT s.state_name, g.good_name, g.shortage, g.price \
             FROM states s \
             JOIN goods_by_state g USING (state_id) \
             WHERE g.shortage > 0 \
             ORDER BY g.shortage DESC \
             LIMIT 20",
        )
        .await
        .expect("join");
    assert!(!batches.is_empty());
}

#[tokio::test]
async fn player_tag_scopes_owner_states() {
    let eng = engine().await;
    let tag = eng
        .query("SELECT player_tag() AS t")
        .await
        .expect("player_tag");
    let tags = tag[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("t");
    assert!(!tags.is_null(0));
    assert_eq!(tags.value(0), "GER");

    // Short `states` is already player-scoped — no owner_tag filter needed.
    let owned = eng
        .query("SELECT state_id, owner_tag FROM states")
        .await
        .expect("owned states");
    let rows: usize = owned.iter().map(|b| b.num_rows()).sum();
    assert!(rows >= 1, "expected ≥1 player-owned state");
    for batch in &owned {
        let col = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..col.len() {
            assert_eq!(col.value(i), "GER");
        }
    }

    let world = eng
        .query("SELECT COUNT(*) AS n FROM world_states")
        .await
        .expect("world_states");
    assert_eq!(world[0].num_rows(), 1);
    // Plaintext fixture is single-state; multi-country coverage lives in
    // `world_tables_include_foreign_when_present`.

    let joined = eng
        .query(
            "SELECT s.state_id, s.owner_tag \
             FROM states s \
             JOIN goods_by_state g USING (state_id)",
        )
        .await
        .expect("domestic goods join");
    for batch in &joined {
        let col = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..col.len() {
            assert_eq!(col.value(i), "GER");
        }
    }
}

#[tokio::test]
async fn region_name_non_null_when_region_id_set() {
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT count(*) AS n FROM states \
             WHERE region_id IS NOT NULL AND region_name IS NULL",
        )
        .await
        .expect("null region_name");
    let count = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int64Array>()
        .expect("count")
        .value(0);
    assert_eq!(count, 0, "region_name must fall back when region_id is set");
}

#[tokio::test]
async fn state_name_non_null_when_region_id_set() {
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT count(*) AS n FROM states \
             WHERE region_id IS NOT NULL AND state_name IS NULL",
        )
        .await
        .expect("null state_name");
    let count = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int64Array>()
        .expect("count")
        .value(0);
    assert_eq!(count, 0, "state_name must be set when region_id is set");
}

#[tokio::test]
async fn pops_profession_not_pop_type_or_state_pops() {
    let eng = engine().await;
    let ok = eng
        .query("SELECT profession FROM pops LIMIT 1")
        .await
        .expect("profession");
    assert_eq!(ok[0].schema().field(0).name(), "profession");

    let pop_type = eng.query("SELECT pop_type FROM pops LIMIT 1").await;
    assert!(pop_type.is_err(), "pop_type must not be a SQL column");

    let state_pops = eng.query("SELECT * FROM state_pops LIMIT 1").await;
    assert!(state_pops.is_err(), "state_pops must not be a SQL table");
}

#[tokio::test]
async fn alerts_lean_select_honors_limit() {
    let eng = engine().await;
    let batches = eng
        .query("SELECT kind, title FROM alerts() LIMIT 5")
        .await
        .expect("lean alerts");
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert!(rows <= 5, "expected at most 5 rows, got {rows}");
    assert_eq!(batches[0].schema().field(0).name(), "kind");
    assert_eq!(batches[0].schema().field(1).name(), "title");
}

#[tokio::test]
async fn alerts_evidence_projection_skips_mitigations_builders() {
    // Evidence alone must not force the mitigations path (empty lists in lean result).
    // Behavioral check: query succeeds and returns evidence JSON; a follow-up that
    // projects mitigations for a filtered LIMIT still works.
    let eng = engine().await;
    let evidence_only = eng
        .query("SELECT id, evidence FROM alerts() LIMIT 5")
        .await
        .expect("evidence-only alerts");
    let n: usize = evidence_only.iter().map(|b| b.num_rows()).sum();
    assert!(n <= 5);
    assert_eq!(evidence_only[0].schema().field(1).name(), "evidence");

    let with_mit = eng
        .query(
            "SELECT id, mitigations FROM alerts() \
             WHERE kind = 'goods_shortage' LIMIT 1",
        )
        .await
        .expect("mitigations after lean");
    let mit_n: usize = with_mit.iter().map(|b| b.num_rows()).sum();
    assert!(mit_n <= 1);
    if mit_n > 0 {
        assert_eq!(with_mit[0].schema().field(1).name(), "mitigations");
        let col = with_mit[0]
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("mitigations text");
        // Lean path would leave "[]"; filtered expand may be "[]" or a JSON array.
        assert!(col.value(0).starts_with('['));
    }
}

#[tokio::test]
async fn alerts_severity_and_kind_filters() {
    let eng = engine().await;
    let all = eng
        .query("SELECT kind FROM alerts()")
        .await
        .expect("all kinds");
    let all_n: usize = all.iter().map(|b| b.num_rows()).sum();

    let filtered = eng
        .query("SELECT kind, severity FROM alerts() WHERE severity = 1 AND kind = 'goods_shortage'")
        .await
        .expect("filtered");
    let filtered_n: usize = filtered.iter().map(|b| b.num_rows()).sum();
    assert!(
        filtered_n <= all_n,
        "filtered ({filtered_n}) must be ≤ unfiltered ({all_n})"
    );
    for batch in &filtered {
        let kinds = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let sev = batch
            .column(1)
            .as_any()
            .downcast_ref::<datafusion::arrow::array::Int32Array>()
            .unwrap();
        for i in 0..batch.num_rows() {
            assert_eq!(kinds.value(i), "goods_shortage");
            assert_eq!(sev.value(i), 1);
        }
    }
}

#[tokio::test]
async fn alerts_limit_applies_with_kind_filter_before_mitigations() {
    // Documented interaction: Exact kind filter + LIMIT + mitigations projection
    // must return at most N rows (LIMIT applied after filter, before/with selective
    // mitigation builders — not after building every mitigation).
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT id, kind, mitigations FROM alerts() \
             WHERE kind = 'unfilled_education' LIMIT 2",
        )
        .await
        .expect("limit+filter mitigations");
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert!(rows <= 2, "expected at most 2 rows, got {rows}");
    for batch in &batches {
        let kinds = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            assert_eq!(kinds.value(i), "unfilled_education");
        }
    }
}

#[tokio::test]
async fn alerts_default_is_player_scoped() {
    let eng = engine().await;

    let player_states = eng
        .query("SELECT state_id FROM states")
        .await
        .expect("player states");
    let mut owned = std::collections::BTreeSet::new();
    for batch in &player_states {
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<UInt32Array>()
            .expect("state_id");
        for i in 0..col.len() {
            owned.insert(col.value(i));
        }
    }
    assert!(!owned.is_empty(), "fixture player GER should own states");

    let scoped = eng
        .query("SELECT state_id FROM alerts()")
        .await
        .expect("alerts()");
    let all = eng
        .query("SELECT state_id FROM alerts('all')")
        .await
        .expect("alerts('all')");
    let scoped_n: usize = scoped.iter().map(|b| b.num_rows()).sum();
    let all_n: usize = all.iter().map(|b| b.num_rows()).sum();
    assert!(
        scoped_n <= all_n,
        "alerts() ({scoped_n}) must be ≤ alerts('all') ({all_n})"
    );
    assert!(all_n > 0, "alerts('all') should still return volume");

    for batch in &scoped {
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<UInt32Array>()
            .expect("state_id");
        for i in 0..col.len() {
            if col.is_null(i) {
                continue;
            }
            let sid = col.value(i);
            assert!(
                owned.contains(&sid),
                "alerts() state_id {sid} must be player-owned"
            );
        }
    }

    let bad = eng.query("SELECT * FROM alerts('domestic')").await;
    assert!(bad.is_err(), "unknown alerts() arg must plan_err");
}

#[tokio::test]
async fn suggest_mitigations_player_le_all_and_columns() {
    let eng = engine().await;

    let player = eng
        .query("SELECT * FROM suggest_mitigations()")
        .await
        .expect("suggest_mitigations()");
    let player_alias = eng
        .query("SELECT * FROM suggest_mitigations('player')")
        .await
        .expect("suggest_mitigations('player')");
    let all = eng
        .query("SELECT * FROM suggest_mitigations('all')")
        .await
        .expect("suggest_mitigations('all')");

    let player_n: usize = player.iter().map(|b| b.num_rows()).sum();
    let alias_n: usize = player_alias.iter().map(|b| b.num_rows()).sum();
    let all_n: usize = all.iter().map(|b| b.num_rows()).sum();
    assert_eq!(player_n, alias_n, "() and ('player') must match");
    assert!(
        player_n <= all_n,
        "player ({player_n}) must be ≤ all ({all_n})"
    );
    assert!(all_n > 0, "fixture should yield mitigations");

    let schema = all[0].schema();
    let expected = [
        "alert_id",
        "mitigation_id",
        "state_id",
        "kind",
        "rank",
        "action",
        "building",
        "good_name",
        "extra_levels",
        "title",
        "detail",
    ];
    assert_eq!(schema.fields().len(), expected.len());
    for (i, name) in expected.iter().enumerate() {
        assert_eq!(schema.field(i).name(), *name);
    }

    // Smoke: detail is JSON; at least one row has a non-empty action or title.
    let titles = all[0]
        .column(9)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("title");
    let details = all[0]
        .column(10)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("detail");
    assert!(!titles.value(0).is_empty());
    assert!(details.value(0).starts_with('{'));

    let bad = eng
        .query("SELECT * FROM suggest_mitigations('domestic')")
        .await;
    assert!(
        bad.is_err(),
        "unknown suggest_mitigations() arg must plan_err"
    );
}

#[tokio::test]
async fn rejects_ddl() {
    let eng = engine().await;
    let err = eng.query("CREATE TABLE t (a INT)").await.expect_err("ddl");
    assert!(matches!(err, SqlError::ReadOnly(_)));
}

#[test]
fn sync_query_helper_works() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let eng = rt.block_on(engine());
    let batches = query(&eng, "SELECT COUNT(*) AS n FROM countries").expect("count");
    assert_eq!(batches[0].num_rows(), 1);
}

#[tokio::test]
async fn btree_range_on_building_types() {
    let eng = engine().await;
    // Exact range pushdown on BTreeMap key `type_id`.
    let batches = eng
        .query(
            "SELECT type_id FROM building_types \
             WHERE type_id >= 'building_a' AND type_id < 'building_z'",
        )
        .await
        .expect("range");
    for batch in &batches {
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..col.len() {
            let v = col.value(i);
            assert!(("building_a".."building_z").contains(&v), "{v}");
        }
    }
}

#[tokio::test]
async fn diagnostics_alerts_and_good_price() {
    let eng = engine().await;
    let alerts = eng
        .query("SELECT id, kind, severity, title, evidence, mitigations FROM alerts()")
        .await
        .expect("alerts");
    assert!(!alerts.is_empty());
    let schema = alerts[0].schema();
    assert_eq!(schema.field(0).name(), "id");
    assert_eq!(schema.field(4).name(), "evidence");
    assert_eq!(schema.field(5).name(), "mitigations");

    let priced = eng
        .query("SELECT good_price('grain') AS p")
        .await
        .expect("good_price");
    let p = priced[0]
        .column(0)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("p");
    assert!(!p.is_null(0));
    assert!(p.value(0).is_finite());

    let unknown = eng
        .query("SELECT good_price('not_a_real_good') AS p")
        .await
        .expect("unknown good");
    let u = unknown[0]
        .column(0)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert!(u.is_null(0));

    let army = eng.query("SELECT army_power() AS a").await;
    // Fixture plaintext has a player but no PP fields → hard error (not NULL/0).
    let err = army.expect_err("army_power should error when projection unknown");
    let msg = err.to_string();
    assert!(
        msg.contains("army power projection unknown") || msg.contains("army_power()"),
        "{msg}"
    );
}

#[tokio::test]
async fn is_underemployed_matches_alerts() {
    let eng = engine().await;

    let null_row = eng
        .query("SELECT is_underemployed(NULL) AS u")
        .await
        .expect("null arg");
    let null_col = null_row[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .expect("bool");
    assert!(null_col.is_null(0));

    let under = eng
        .query(
            "SELECT state_id FROM alerts('all') \
             WHERE kind = 'underemployed' AND state_id IS NOT NULL \
             ORDER BY state_id LIMIT 1",
        )
        .await
        .expect("underemployed alert");
    assert!(
        !under.is_empty() && under[0].num_rows() > 0,
        "plaintext fixture should have at least one underemployed state"
    );
    let sid = under[0]
        .column(0)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap()
        .value(0);

    let yes = eng
        .query(&format!("SELECT is_underemployed({sid}) AS u"))
        .await
        .expect("true case");
    let yes_col = yes[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(!yes_col.is_null(0));
    assert!(yes_col.value(0), "state {sid} has underemployed alert");

    // State present in `states` but without an underemployed alert (anti-join).
    let other = eng
        .query(
            "SELECT s.state_id FROM states s \
             WHERE NOT EXISTS ( \
               SELECT 1 FROM alerts('all') a \
               WHERE a.kind = 'underemployed' AND a.state_id = s.state_id \
             ) \
             ORDER BY s.state_id LIMIT 1",
        )
        .await
        .expect("non-underemployed state");
    let other_sid = if !other.is_empty() && other[0].num_rows() > 0 {
        other[0]
            .column(0)
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap()
            .value(0)
    } else {
        // Every fixture state is underemployed — pick an id with no alert.
        9_999_999
    };
    let no = eng
        .query(&format!("SELECT is_underemployed({other_sid}) AS u"))
        .await
        .expect("false scalar");
    let no_col = no[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(!no_col.is_null(0));
    assert!(
        !no_col.value(0),
        "state {other_sid} should not be underemployed"
    );

    // Columnar path: UInt32 `states.state_id` coerces into the Int64 UDF.
    let col = eng
        .query(&format!(
            "SELECT is_underemployed(state_id) AS u FROM states WHERE state_id = {sid}"
        ))
        .await
        .expect("columnar");
    let col_bool = col[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert_eq!(col[0].num_rows(), 1);
    assert!(col_bool.value(0));
}

/// End-to-end player scope: `is_underemployed` + `suggest_mitigations()` on short `states`
/// without `owner_tag = player_tag()`.
#[tokio::test]
async fn underemployed_states_join_suggest_mitigations() {
    let eng = engine().await;

    let under = eng
        .query("SELECT state_id FROM states WHERE is_underemployed(state_id) ORDER BY state_id")
        .await
        .expect("underemployed via states");
    let under_n: usize = under.iter().map(|b| b.num_rows()).sum();
    assert!(
        under_n > 0,
        "plaintext fixture should have underemployed player states"
    );

    let joined = eng
        .query(
            "SELECT s.state_id, s.owner_tag, m.kind, m.action, m.title \
             FROM states s \
             JOIN suggest_mitigations() m USING (state_id) \
             WHERE is_underemployed(s.state_id) \
             ORDER BY s.state_id, m.rank \
             LIMIT 40",
        )
        .await
        .expect("join suggest_mitigations on underemployed states");

    let mut saw_underemployed_kind = false;
    for batch in &joined {
        let owners = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("owner_tag");
        let kinds = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("kind");
        for i in 0..batch.num_rows() {
            assert_eq!(
                owners.value(i),
                "GER",
                "short states is player-scoped; no owner_tag filter needed"
            );
            if kinds.value(i) == "underemployed" {
                saw_underemployed_kind = true;
            }
        }
    }
    let joined_n: usize = joined.iter().map(|b| b.num_rows()).sum();
    assert!(
        joined_n > 0 && saw_underemployed_kind,
        "expected underemployed mitigations joined to player states (got {joined_n} rows)"
    );
}

#[tokio::test]
async fn shortage_analysis_schema_and_filter() {
    let eng = engine().await;
    let all = eng
        .query("SELECT good_name, alert_id, kind, shortage, evidence FROM shortage_analysis(NULL)")
        .await
        .expect("shortage all");
    assert!(!all.is_empty());
    let schema = all[0].schema();
    assert_eq!(schema.field(0).name(), "good_name");
    assert_eq!(schema.field(3).name(), "shortage");

    let grain = eng
        .query("SELECT good_name FROM shortage_analysis('grain')")
        .await
        .expect("shortage grain");
    for batch in &grain {
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..col.len() {
            assert_eq!(col.value(i), "grain");
        }
    }
}

#[tokio::test]
async fn building_staffing_for_state() {
    let eng = engine().await;
    let states = eng
        .query("SELECT state_id FROM states ORDER BY state_id LIMIT 1")
        .await
        .expect("state");
    let sid = states[0]
        .column(0)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap()
        .value(0);
    let sql = format!(
        "SELECT building_id, type_id, profession_id, employed_here, jobs_here \
         FROM building_staffing({sid})"
    );
    let batches = eng.query(&sql).await.expect("staffing");
    assert!(!batches.is_empty());
    assert_eq!(batches[0].schema().field(0).name(), "building_id");
}

#[test]
fn binding_arcs_are_shareable() {
    let defs = decode_blob(&defs_blob()).unwrap();
    let save = load_slice(&save_bytes(), empty_tokens()).unwrap();
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, SolveOpts::default());
    let b = Arc::new(vic3_sql::SessionBinding::new(defs, world, prices));
    assert!(!b.prices.goods.is_empty());
}

#[tokio::test]
async fn plan_research_returns_ordered_steps() {
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT step, day, action, detail FROM plan('research(tech=nitroglycerin)') \
             ORDER BY step",
        )
        .await
        .expect("plan");
    assert!(!batches.is_empty());
    let batch = &batches[0];
    assert!(
        batch.num_rows() >= 2,
        "expected queue + wait, got {}",
        batch.num_rows()
    );

    let steps = batch
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Int32Array>()
        .expect("step");
    let days = batch
        .column(1)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .expect("day");
    let actions = batch
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("action");

    assert_eq!(steps.value(0), 0);
    assert_eq!(actions.value(0), "QueueTech");
    assert!(actions.value(1).contains("Wait") || actions.value(1) == "WaitForEvent");
    assert!(days.value(batch.num_rows() - 1) > 0);

    // Column contract including nullable limitations on the full projection.
    let full = eng
        .query("SELECT step, day, action, detail, limitations FROM plan('research(tech=nitroglycerin)')")
        .await
        .expect("plan full");
    assert_eq!(full[0].schema().fields().len(), 5);
    assert_eq!(full[0].schema().field(0).name(), "step");
    assert_eq!(full[0].schema().field(4).name(), "limitations");
}

#[tokio::test]
async fn plan_accepts_max_days_and_label() {
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT step FROM plan('research(tech=nitroglycerin)', 3650, 'fixture') \
             ORDER BY step",
        )
        .await
        .expect("plan opts");
    assert!(batches[0].num_rows() >= 1);
}

#[tokio::test]
async fn gaps_research_marks_tech_predicate() {
    let eng = engine().await;
    let batches = eng
        .query("SELECT predicate, status, detail FROM gaps('research(tech=nitroglycerin)')")
        .await
        .expect("gaps");
    assert_eq!(batches[0].num_rows(), 1);
    let preds = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("predicate");
    let statuses = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("status");
    assert_eq!(preds.value(0), "has_tech(nitroglycerin)");
    assert!(
        statuses.value(0) == "failing" || statuses.value(0) == "cleared",
        "{}",
        statuses.value(0)
    );
    assert_eq!(batches[0].schema().field(0).name(), "predicate");
    assert_eq!(batches[0].schema().field(1).name(), "status");
    assert_eq!(batches[0].schema().field(2).name(), "detail");
}

#[tokio::test]
async fn gaps_declare_war_exposes_interest_army_munitions_solvent() {
    let eng = engine().await;
    let batches = eng
        .query(
            "SELECT predicate, status FROM gaps(\
             'declare-war(tag=FRA, wargoal=conquer_state, state=alsace)')",
        )
        .await
        .expect("gaps war");
    assert_eq!(batches[0].num_rows(), 4);
    let preds = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("predicate");
    let mut seen = std::collections::HashSet::new();
    for i in 0..preds.len() {
        seen.insert(preds.value(i).to_string());
    }
    assert!(seen.iter().any(|p| p.starts_with("interest_in")));
    assert!(seen.iter().any(|p| p.starts_with("army_power_projection")));
    assert!(seen.iter().any(|p| p.starts_with("good_price(ammunition)")));
    assert!(seen.contains("solvent"));
}

#[tokio::test]
async fn gaps_army_power_unknown_not_silent_zero() {
    let eng = engine().await;
    // Fixture plaintext has a player but no PP fields (same as army_power() error).
    let batches = eng
        .query("SELECT predicate, status FROM gaps('army_power_projection >= 100')")
        .await
        .expect("gaps army");
    assert_eq!(batches[0].num_rows(), 1);
    let preds = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("predicate");
    let statuses = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("status");
    assert_eq!(preds.value(0), "army_power_projection >= 100");
    assert_eq!(
        statuses.value(0),
        "unknown",
        "missing PP must not look like measured shortfall"
    );
}

#[tokio::test]
async fn constructions_table_lists_private_and_government() {
    use vic3_prices::{ConstructionQueueKind, WorldConstruction};

    let defs = decode_blob(&defs_blob()).expect("decode defs");
    let save = load_slice(&save_bytes(), empty_tokens()).expect("load save");
    let mut world = World::from_save(&save, &defs);
    let country_id = world
        .countries
        .iter()
        .find(|c| c.tag == "GER")
        .map(|c| c.id);
    world.constructions = vec![
        WorldConstruction {
            id: 10,
            queue: ConstructionQueueKind::Private,
            country_id,
            state_id: Some(1),
            building: "building_logging_camp".into(),
            remaining: Some(5.0),
        },
        WorldConstruction {
            id: 1,
            queue: ConstructionQueueKind::Government,
            country_id,
            state_id: Some(1),
            building: "building_construction_sector".into(),
            remaining: Some(40.0),
        },
    ];
    let prices = solve(&world, &defs, SolveOpts::default());
    let eng = SqlEngine::bind(defs, world, prices).await.expect("bind");

    let batches = eng
        .query(
            "SELECT queue, position, building, remaining FROM constructions ORDER BY queue, position",
        )
        .await
        .expect("constructions");
    assert_eq!(batches[0].num_rows(), 2);
    let queues = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("queue");
    // Utf8 sort: government before private.
    assert_eq!(queues.value(0), "government");
    assert_eq!(queues.value(1), "private");

    let gov = eng
        .query("SELECT building FROM constructions WHERE queue = 'government'")
        .await
        .expect("gov filter");
    let buildings = gov[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("building");
    assert_eq!(gov[0].num_rows(), 1);
    assert_eq!(buildings.value(0), "building_construction_sector");
}
