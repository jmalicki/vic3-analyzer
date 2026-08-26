//! Wall times for alerts lean vs fat on a live save (issue #37).
//!
//! Separates **mitigation compute** from **JSON payload size** so we do not
//! conflate timeouts-from-slowness with timeouts-from-bytes.
//!
//! ```text
//! VIC3_SAVE=… VIC3_TOKENS=… VIC3_DEFS=… \
//!   cargo test -p vic3-prices --release --test live_alerts_timing -- --ignored --nocapture
//! ```
//!
//! Prefer the GER 1907 save cited in #37:
//! `…/save games/germany_1907_01_18.v3`

use std::time::{Duration, Instant};

use vic3_load::{empty_tokens, load_path_world, load_tokens_path};
use vic3_prices::{alerts_with, AlertsOptions, AlertsResult, SolveOpts, World};

#[test]
#[ignore = "set VIC3_SAVE (and VIC3_TOKENS for binary) plus VIC3_GAME or VIC3_DEFS"]
fn live_alerts_lean_vs_fat() {
    let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE");
    let defs = if let Ok(path) = std::env::var("VIC3_DEFS") {
        vic3_defs::decode_blob(&std::fs::read(path).expect("defs blob")).expect("decode blob")
    } else {
        vic3_defs::load_from_path(std::env::var("VIC3_GAME").expect("VIC3_GAME"))
            .expect("game defs")
    };

    let load_started = Instant::now();
    let tokens = match std::env::var("VIC3_TOKENS") {
        Ok(path) => load_tokens_path(path).expect("tokens"),
        Err(_) => empty_tokens(),
    };
    let save = load_path_world(&save_path, tokens).expect("load save");
    let parse_elapsed = load_started.elapsed();
    let world = World::from_save(&save, &defs);
    drop(save);

    let solve_started = Instant::now();
    let prices = vic3_prices::solve(&world, &defs, SolveOpts::default());
    let solve_elapsed = solve_started.elapsed();
    eprintln!(
        "parse {:?}  solve {:?}  buildings {}  pops {}  goods {}",
        parse_elapsed,
        solve_elapsed,
        world.buildings.len(),
        world.pop_count(),
        defs.goods_order.len()
    );

    // Cold compute: detectors only (empty mitigations lists).
    let lean = time_runs("alerts_with(mitigations=false) compute", 3, || {
        alerts_with(
            &world,
            &defs,
            &prices,
            AlertsOptions {
                with_mitigations: false,
                mitigation_ids: None,
            },
        )
    });

    // Cold compute: full mitigation builders (world clones / effect strings).
    let fat = time_runs("alerts_with(mitigations=true) compute", 3, || {
        alerts_with(
            &world,
            &defs,
            &prices,
            AlertsOptions {
                with_mitigations: true,
                mitigation_ids: None,
            },
        )
    });

    summarize_result("lean", &lean);
    summarize_result("fat", &fat);

    // Serialization cost of MCP-like payloads (after compute already done).
    let lean_json = time_serde("lean AlertsResult JSON", &lean);
    let fat_json = time_serde("fat AlertsResult JSON", &fat);
    eprintln!(
        "payload lean {} bytes ({:.2} MiB)  fat {} bytes ({:.2} MiB)  ratio {:.1}x",
        lean_json.len(),
        lean_json.len() as f64 / (1024.0 * 1024.0),
        fat_json.len(),
        fat_json.len() as f64 / (1024.0 * 1024.0),
        fat_json.len() as f64 / lean_json.len().max(1) as f64
    );

    // Approximate SQL/MCP shape: identity + JSON text columns per row.
    let lean_sql = time_sql_shape("lean SQL-shaped rows JSON", &lean);
    let fat_sql = time_sql_shape("fat SQL-shaped rows JSON", &fat);
    eprintln!(
        "sql-shaped lean {} bytes ({:.2} MiB)  fat {} bytes ({:.2} MiB)  ratio {:.1}x",
        lean_sql.len(),
        lean_sql.len() as f64 / (1024.0 * 1024.0),
        fat_sql.len(),
        fat_sql.len() as f64 / (1024.0 * 1024.0),
        fat_sql.len() as f64 / lean_sql.len().max(1) as f64
    );

    // Evidence-only vs mitigations bytes inside fat result (size attribution).
    let mut evidence_bytes = 0usize;
    let mut mitigations_bytes = 0usize;
    for alert in &fat.alerts {
        evidence_bytes += serde_json::to_vec(&alert.evidence)
            .map(|v| v.len())
            .unwrap_or(0);
        mitigations_bytes += serde_json::to_vec(&alert.mitigations)
            .map(|v| v.len())
            .unwrap_or(0);
    }
    eprintln!(
        "inside fat rows: evidence_json_sum {} bytes ({:.2} MiB)  mitigations_json_sum {} bytes ({:.2} MiB)",
        evidence_bytes,
        evidence_bytes as f64 / (1024.0 * 1024.0),
        mitigations_bytes,
        mitigations_bytes as f64 / (1024.0 * 1024.0)
    );
}

fn summarize_result(label: &str, result: &AlertsResult) {
    let mut mitigations = 0usize;
    let mut evidence = 0usize;
    for alert in &result.alerts {
        mitigations += alert.mitigations.len();
        evidence += alert.evidence.len();
    }
    eprintln!(
        "{label}: {} alerts  {} evidence entries  {} mitigations  limitations {}",
        result.alerts.len(),
        evidence,
        mitigations,
        result.limitations.len()
    );
}

fn time_serde(label: &str, result: &AlertsResult) -> Vec<u8> {
    let started = Instant::now();
    let bytes = serde_json::to_vec(result).expect("serialize AlertsResult");
    eprintln!("{label} {:?} -> {} bytes", started.elapsed(), bytes.len());
    bytes
}

fn time_sql_shape(label: &str, result: &AlertsResult) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct Row<'a> {
        id: &'a str,
        kind: String,
        severity: u8,
        title: &'a str,
        summary: &'a str,
        state_id: Option<u32>,
        building_id: Option<u32>,
        good_name: Option<&'a str>,
        evidence: String,
        mitigations: String,
    }
    let started = Instant::now();
    let rows: Vec<Row<'_>> = result
        .alerts
        .iter()
        .map(|a| Row {
            id: &a.id,
            kind: format!("{:?}", a.kind),
            severity: a.severity,
            title: &a.title,
            summary: &a.summary,
            state_id: a.state_id,
            building_id: a.building_id,
            good_name: a.good_name.as_deref(),
            evidence: serde_json::to_string(&a.evidence).unwrap_or_else(|_| "[]".into()),
            mitigations: serde_json::to_string(&a.mitigations).unwrap_or_else(|_| "[]".into()),
        })
        .collect();
    let payload = serde_json::json!({
        "columns": ["id","kind","severity","title","summary","state_id","building_id", "good_name","evidence","mitigations"],
        "rows": rows,
        "row_count": rows.len(),
    });
    let bytes = serde_json::to_vec(&payload).expect("serialize sql-shaped");
    eprintln!("{label} {:?} -> {} bytes", started.elapsed(), bytes.len());
    bytes
}

fn time_runs<T>(label: &str, n: usize, mut run: impl FnMut() -> T) -> T {
    let mut last = None;
    let mut times = Vec::new();
    for i in 0..n {
        let started = Instant::now();
        last = Some(run());
        let elapsed = started.elapsed();
        times.push(elapsed);
        eprintln!("{label} run {} {:?}", i + 1, elapsed);
    }
    times.sort();
    eprintln!("{label} median {:?}", median(&times));
    last.expect("n > 0")
}

fn median(times: &[Duration]) -> Duration {
    times[times.len() / 2]
}
