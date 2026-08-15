//! In-browser facade: bytes in, JSON strings out. No filesystem, no clap.
//!
//! Build for the browser with:
//! ```text
//! wasm-pack build crates/vic3-wasm --target web --out-dir web/public/wasm
//! ```
//! (or `npm run build:wasm` from `web/`). The crate already compiles for
//! `wasm32-unknown-unknown` without extra `getrandom` feature flags on the
//! current dependency set.
//!
//! wasm-bindgen exports return JSON text (not `JsValue`) so native tests can
//! round-trip the same payload the CLI prints.

mod error;
mod schema;
mod world;

use serde::Serialize;
use vic3_goals::Atom;
use vic3_load::{empty_tokens, load_slice, load_tokens_slice, Save};
use vic3_plan::PlanOpts;
use vic3_prices::{solve, what_if as solve_what_if, PricesResult, SolveOpts, WhatIfOpts};
use vic3_sim::SimConfig;
use vic3_world::PlanningState;
use vic3save::PdsDate;
use wasm_bindgen::prelude::*;

pub use error::WasmError;

/// Crate version from Cargo.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Parse a `.v3` (and optional token map) into an IR summary JSON.
///
/// Empty / omitted `tokens_bytes` is correct for plaintext. Binary saves with
/// no tokens return [`vic3_load::LoadError::MissingTokens`].
#[wasm_bindgen]
pub fn parse_save(save_bytes: &[u8], tokens_bytes: Option<Vec<u8>>) -> Result<String, JsError> {
    parse_save_json(save_bytes, tokens_bytes.as_deref()).map_err(to_js)
}

/// Solve market prices. `defs_blob` is a postcard blob from [`vic3_defs::encode_blob`].
/// `solve_opts_json` is a [`SolveOpts`] object; empty / `{}` uses defaults.
#[wasm_bindgen]
pub fn prices(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, JsError> {
    prices_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
    )
    .map_err(to_js)
}

/// Apply a what-if building-level delta and re-solve.
///
/// `what_if_opts_json` is [`WhatIfOpts`] (`building`, `extra_levels`).
#[wasm_bindgen]
pub fn what_if(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    what_if_opts_json: &str,
) -> Result<String, JsError> {
    what_if_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
        what_if_opts_json,
    )
    .map_err(to_js)
}

/// JSON Schema for [`WhatIfOpts`] (UI form).
#[wasm_bindgen]
pub fn what_if_schema() -> String {
    schema::what_if_schema_json()
}

/// JSON Schema for [`PricesResult`] (includes `residual` and `limitations`).
#[wasm_bindgen]
pub fn prices_schema() -> String {
    schema::prices_schema_json()
}

/// Find a shortest goal-relevant plan and return [`vic3_plan::PlanResult`] JSON.
#[wasm_bindgen]
pub fn plan(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    plan_opts_json: &str,
) -> Result<String, JsError> {
    plan_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
        plan_opts_json,
    )
    .map_err(to_js)
}

/// Evaluate a goal and return unsatisfied atoms (`GapsResult` JSON, CLI parity).
#[wasm_bindgen]
pub fn gaps(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    goal: &str,
) -> Result<String, JsError> {
    gaps_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
        goal,
    )
    .map_err(to_js)
}

/// Native/test entry: same JSON as [`parse_save`].
pub fn parse_save_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
) -> Result<String, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    Ok(serde_json::to_string(&SaveSummary::from(&save))?)
}

/// Native/test entry: same `PricesResult` JSON as the CLI.
pub fn prices_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, WasmError> {
    let result = run_prices(save_bytes, tokens_bytes, defs_blob, solve_opts_json)?;
    Ok(serde_json::to_string(&result)?)
}

/// Native/test entry: same `PricesResult` JSON as the CLI `what-if` command.
pub fn what_if_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    what_if_opts_json: &str,
) -> Result<String, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let delta: WhatIfOpts = serde_json::from_str(what_if_opts_json)?;
    let world = world::world_from_save(&save);
    let result = solve_what_if(&world, &defs, &delta, opts);
    Ok(serde_json::to_string(&result)?)
}

/// Native/test entry: same `PlanResult` JSON as the CLI `plan` command.
pub fn plan_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    plan_opts_json: &str,
) -> Result<String, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let solve_opts = parse_solve_opts(solve_opts_json)?;
    let plan_opts: PlanOpts = serde_json::from_str(plan_opts_json)?;
    let world = world::world_from_save(&save);
    let prices = solve(&world, &defs, solve_opts);
    let country = country_tag(&save)?;
    let state = PlanningState::from_save(&save, country, &prices)?;
    let goal = vic3_goals::parse(&plan_opts.goal)?;
    let result = vic3_plan::plan(
        state,
        goal,
        SimConfig::default(),
        plan_opts.max_days,
        prices.residual,
        prices.limitations,
    )?;
    Ok(serde_json::to_string(&result)?)
}

/// Native/test entry: same `GapsResult` JSON as the CLI `gaps` command.
pub fn gaps_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    goal: &str,
) -> Result<String, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let world = world::world_from_save(&save);
    let prices = solve(&world, &defs, opts);
    let country = country_tag(&save)?;
    let state = PlanningState::from_save(&save, country, &prices)?;
    let goal = vic3_goals::parse(goal)?;
    let result = GapsResult {
        satisfied: vic3_goals::evaluate(&goal, &state),
        gaps: vic3_goals::gaps(&goal, &state),
        limitations: prices.limitations,
    };
    Ok(serde_json::to_string(&result)?)
}

fn country_tag(save: &Save) -> Result<&str, WasmError> {
    save.previous_played
        .iter()
        .find_map(|player| player.name.as_deref())
        .or_else(|| {
            save.countries()
                .next()
                .map(|(_, country)| country.definition.as_str())
        })
        .ok_or(WasmError::NoCountry)
}

/// CLI-parity gaps payload (`satisfied`, `gaps`, `limitations`).
#[derive(Debug, Serialize)]
struct GapsResult {
    satisfied: bool,
    gaps: Vec<Atom>,
    limitations: Vec<String>,
}

fn run_prices(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<PricesResult, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let world = world::world_from_save(&save);
    Ok(solve(&world, &defs, opts))
}

fn parse_solve_opts(json: &str) -> Result<SolveOpts, WasmError> {
    let json = json.trim();
    if json.is_empty() {
        return Ok(SolveOpts::default());
    }
    Ok(serde_json::from_str(json)?)
}

fn load_save(save_bytes: &[u8], tokens_bytes: Option<&[u8]>) -> Result<Save, WasmError> {
    match tokens_bytes {
        None | Some([]) => Ok(load_slice(save_bytes, empty_tokens())?),
        Some(tokens) => {
            let resolver = load_tokens_slice(tokens)?;
            Ok(load_slice(save_bytes, resolver)?)
        }
    }
}

fn to_js(err: WasmError) -> JsError {
    JsError::new(&err.to_string())
}

#[derive(Debug, Serialize)]
struct SaveSummary {
    tag: Option<String>,
    date: Option<String>,
    version: String,
    counts: SaveCounts,
}

#[derive(Debug, Serialize)]
struct SaveCounts {
    countries: usize,
    states: usize,
    buildings: usize,
    pops: usize,
    markets: usize,
    trade_routes: usize,
}

impl From<&Save> for SaveSummary {
    fn from(save: &Save) -> Self {
        let tag = save
            .previous_played
            .iter()
            .find_map(|player| player.name.clone())
            .or_else(|| {
                save.countries()
                    .next()
                    .map(|(_, country)| country.definition.clone())
            });
        let date = save.meta_data.game_date.map(|d| d.game_fmt().to_string());
        Self {
            tag,
            date,
            version: save.meta_data.version.clone(),
            counts: SaveCounts {
                countries: save.country_manager.iter_present().count(),
                states: save.states.iter_present().count(),
                buildings: save.building_manager.iter_present().count(),
                pops: save.pops.iter_present().count(),
                markets: save.market_manager.iter_present().count(),
                trade_routes: save.trade_route_manager.iter_present().count(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;
    use vic3_defs::{encode_blob, load_from_path};
    use vic3_load::LoadError;
    use vic3_prices::SolveStatus;

    fn load_fixture() -> Vec<u8> {
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../vic3-load/tests/fixtures/plaintext.txt"),
        )
        .expect("plaintext fixture")
    }

    fn barren_fixture() -> Vec<u8> {
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-cli/tests/fixtures/barren.txt"),
        )
        .expect("barren fixture")
    }

    fn defs_blob() -> Vec<u8> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures");
        let defs = load_from_path(&root).expect("defs fixture");
        encode_blob(&defs).expect("encode defs blob")
    }

    fn schema_properties(schema_json: &str) -> serde_json::Map<String, Value> {
        let v: Value = serde_json::from_str(schema_json).expect("schema json");
        v.get("properties")
            .and_then(Value::as_object)
            .cloned()
            .expect("schema properties")
    }

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn parse_save_plaintext_summary() {
        let json = parse_save_json(&load_fixture(), None).expect("parse plaintext");
        let v: Value = serde_json::from_str(&json).expect("summary json");
        assert_eq!(v["tag"], "GER");
        assert_eq!(v["date"], "1836.1.1");
        assert_eq!(v["version"], "1.9.0");
        assert_eq!(v["counts"]["countries"], 1);
        assert_eq!(v["counts"]["states"], 1);
        assert_eq!(v["counts"]["buildings"], 1);
        assert_eq!(v["counts"]["pops"], 1);
        assert_eq!(v["counts"]["markets"], 1);
        assert_eq!(v["counts"]["trade_routes"], 1);
    }

    #[test]
    fn parse_save_binary_without_tokens_is_missing_tokens() {
        let mut bytes = b"SAV0101deadbeef00000000\n".to_vec();
        bytes.extend_from_slice(&[0u8; 32]);
        let err = parse_save_json(&bytes, None).expect_err("binary needs tokens");
        assert!(
            matches!(err, WasmError::Load(LoadError::MissingTokens)),
            "expected MissingTokens, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("token"),
            "error should mention tokens, got: {msg}"
        );
        assert!(
            !msg.to_lowercase().contains("missing field"),
            "must not be a serde mystery: {msg}"
        );
    }

    #[test]
    fn parse_save_binary_empty_tokens_is_missing_tokens() {
        let mut bytes = b"SAV0101deadbeef00000000\n".to_vec();
        bytes.extend_from_slice(&[0u8; 32]);
        let err = parse_save_json(&bytes, Some(&[])).expect_err("empty tokens");
        assert!(matches!(err, WasmError::Load(LoadError::MissingTokens)));
    }

    #[test]
    fn prices_json_has_residual_and_limitations() {
        let json = prices_json(&load_fixture(), None, &defs_blob(), "{}").expect("prices");
        let result: PricesResult = serde_json::from_str(&json).expect("PricesResult");
        assert!(result.residual.is_finite());
        assert!(result.residual >= 0.0);
        assert!(!result.limitations.is_empty());
        assert_eq!(
            result.limitations,
            vic3_prices::LIMITATIONS
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
        assert!(!result.goods.is_empty());
        if result.status == SolveStatus::Converged {
            assert!(result.residual < SolveOpts::default().residual_eps);
        }
    }

    #[test]
    fn what_if_json_has_residual_and_limitations() {
        let json = what_if_json(
            &load_fixture(),
            None,
            &defs_blob(),
            "{}",
            r#"{"building":"building_rye_farm","extra_levels":5}"#,
        )
        .expect("what-if");
        let result: PricesResult = serde_json::from_str(&json).expect("PricesResult");
        assert!(result.residual.is_finite());
        assert!(!result.limitations.is_empty());
        assert_eq!(result.limitations.len(), vic3_prices::LIMITATIONS.len());
        assert!(json.contains("\"residual\""));
        assert!(json.contains("\"limitations\""));
    }

    #[test]
    fn plan_json_contains_timeline_and_default_research_cost() {
        let json = plan_json(
            &load_fixture(),
            None,
            &defs_blob(),
            "{}",
            r#"{"goal":"research(tech=nitroglycerin)","max_days":1000,"label":"rush"}"#,
        )
        .expect("plan");
        let result: vic3_plan::PlanResult = serde_json::from_str(&json).expect("PlanResult");
        assert_eq!(result.day_cost, 365);
        assert_eq!(result.actions.len(), 2);
        assert!(result.residual.is_finite());
        assert!(!result.limitations.is_empty());
    }

    #[test]
    fn gaps_json_matches_cli_shape_on_barren_fixture() {
        let json = gaps_json(
            &barren_fixture(),
            None,
            &defs_blob(),
            "{}",
            "declare-war(tag=FRA, wargoal=conquer_state, state=alsace)",
        )
        .expect("gaps");
        let value: Value = serde_json::from_str(&json).expect("GapsResult JSON");
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
    fn prices_schema_has_residual_and_limitations() {
        let props = schema_properties(&prices_schema());
        assert!(props.contains_key("residual"), "{props:?}");
        assert!(props.contains_key("limitations"), "{props:?}");
        assert!(props.contains_key("goods"), "{props:?}");
        assert!(props.contains_key("status"), "{props:?}");
    }

    #[test]
    fn what_if_schema_describes_building_and_extra_levels() {
        let schema: Value = serde_json::from_str(&what_if_schema()).expect("schema");
        let props = schema["properties"].as_object().expect("properties");
        assert!(props.contains_key("building"), "{props:?}");
        assert!(props.contains_key("extra_levels"), "{props:?}");
        let required = schema["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(required.contains(&"building"));
        assert!(required.contains(&"extra_levels"));
    }
}
