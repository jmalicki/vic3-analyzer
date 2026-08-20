//! Fixture-backed SQL smoke tests (`docs/sql.md`).

use std::path::PathBuf;
use std::sync::Arc;

use datafusion::arrow::array::{Array, Float64Array, StringArray, UInt32Array};
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

    let countries = eng
        .query("SELECT tag FROM countries WHERE tag = 'GER'")
        .await
        .expect("countries");
    let tags = countries[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("tag");
    assert_eq!(tags.value(0), "GER");
}

#[tokio::test]
async fn goods_shortage_and_equality_filter() {
    let eng = engine().await;
    let batches = eng
        .query("SELECT good, buy, sell, shortage FROM goods WHERE good = 'grain'")
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
            "SELECT s.region_name, g.good, g.shortage, g.price \
             FROM states s \
             JOIN goods_by_state g USING (state_id) \
             WHERE s.owner_tag = player_tag() \
               AND g.shortage > 0 \
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

    let owned = eng
        .query("SELECT state_id, owner_tag FROM states WHERE owner_tag = player_tag()")
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

    let joined = eng
        .query(
            "SELECT s.state_id, s.owner_tag \
             FROM states s \
             JOIN goods_by_state g USING (state_id) \
             WHERE s.owner_tag = player_tag()",
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
async fn shortage_analysis_schema_and_filter() {
    let eng = engine().await;
    let all = eng
        .query("SELECT good, alert_id, kind, shortage, evidence FROM shortage_analysis(NULL)")
        .await
        .expect("shortage all");
    assert!(!all.is_empty());
    let schema = all[0].schema();
    assert_eq!(schema.field(0).name(), "good");
    assert_eq!(schema.field(3).name(), "shortage");

    let grain = eng
        .query("SELECT good FROM shortage_analysis('grain')")
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
