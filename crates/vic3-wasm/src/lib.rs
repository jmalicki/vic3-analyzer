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

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeSet;
use vic3_goals::Atom;
use vic3_load::{empty_tokens, load_slice, load_tokens_slice, Save};
use vic3_plan::PlanOpts;
use vic3_prices::{
    alerts as diagnose_alerts, preview as solve_preview, solve, what_if as solve_what_if,
    PricesResult, SolveOpts, WhatIfOpts, World, WorldDelta,
};
use vic3_sim::{EconomyContext, SimConfig};
use vic3_world::PlanningState;
use vic3save::PdsDate;
use wasm_bindgen::prelude::*;

pub use error::WasmError;

struct LoadedAnalysis {
    defs: vic3_defs::GameDefs,
    world: World,
    solve_opts: SolveOpts,
    prices: PricesResult,
    save: Save,
}

thread_local! {
    /// The wasm module lives in the analysis worker, so this singleton has the
    /// same lifetime and isolation as that worker.
    static LOADED_ANALYSIS: RefCell<Option<LoadedAnalysis>> = const { RefCell::new(None) };
}

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

/// Load one worker-owned analysis session and solve its baseline prices.
///
/// The save IR, built world, definitions, and baseline prices remain in wasm
/// for subsequent prices, what-if, gaps, plan, and military calls. Original
/// `.v3` bytes stay in JS. Loading another save replaces the session.
#[wasm_bindgen]
pub fn load_analysis(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, JsError> {
    load_analysis_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
    )
    .map_err(to_js)
}

/// Release the worker-owned save IR, definitions, world, and baseline prices.
#[wasm_bindgen]
pub fn clear_analysis() {
    LOADED_ANALYSIS.with(|loaded| {
        loaded.borrow_mut().take();
    });
}

/// Return the prices computed while loading the current analysis.
#[wasm_bindgen]
pub fn loaded_prices() -> Result<String, JsError> {
    loaded_prices_json().map_err(to_js)
}

/// Return a conservative military snapshot for the played country.
#[wasm_bindgen]
pub fn loaded_military() -> Result<String, JsError> {
    loaded_military_json().map_err(to_js)
}

/// Apply a what-if delta to the current worker-owned world.
#[wasm_bindgen]
pub fn loaded_what_if(what_if_opts_json: &str) -> Result<String, JsError> {
    loaded_what_if_json(what_if_opts_json).map_err(to_js)
}

/// Apply a [`WorldDelta`] to a clone of the loaded world and re-solve (preview).
///
/// Does not replace the loaded world or baseline prices, and does not write a save.
#[wasm_bindgen]
pub fn loaded_apply_delta(delta_json: &str) -> Result<String, JsError> {
    loaded_apply_delta_json(delta_json).map_err(to_js)
}

/// Evaluate goal gaps against the current worker-owned analysis.
#[wasm_bindgen]
pub fn loaded_gaps(goal: &str) -> Result<String, JsError> {
    loaded_gaps_json(goal).map_err(to_js)
}

/// Plan from the current worker-owned analysis.
#[wasm_bindgen]
pub fn loaded_plan(plan_opts_json: &str) -> Result<String, JsError> {
    loaded_plan_json(plan_opts_json).map_err(to_js)
}

/// Diagnose shortages from the current worker-owned analysis (`AlertsResult` JSON).
#[wasm_bindgen]
pub fn loaded_alerts() -> Result<String, JsError> {
    loaded_alerts_json().map_err(to_js)
}

/// Build a postcard definitions blob from browser-selected Victoria 3 files.
///
/// `manifest_json` is an array of `{path, offset, length}` entries into the
/// concatenated `contents` payload. Paths must include `common/...`.
#[wasm_bindgen]
pub fn build_defs_blob(manifest_json: &str, contents: &[u8]) -> Result<Vec<u8>, JsError> {
    build_defs_blob_bytes(manifest_json, contents).map_err(to_js)
}

/// Streaming counterpart to [`build_defs_blob`].
///
/// A full install carries over 400 MB of coat-of-arms art. Handing that to
/// wasm in one array costs the tab roughly a gigabyte and freezes it; feeding
/// batches lets the page read, submit, and release a few files at a time.
#[wasm_bindgen]
#[derive(Default)]
pub struct DefsBlobBuilder {
    inner: vic3_defs::DefsBuilder,
}

#[wasm_bindgen]
impl DefsBlobBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> DefsBlobBuilder {
        DefsBlobBuilder::default()
    }

    /// Absorb one batch, in the same manifest form [`build_defs_blob`] takes.
    #[wasm_bindgen(js_name = addBatch)]
    pub fn add_batch(&mut self, manifest_json: &str, contents: &[u8]) -> Result<(), JsError> {
        let files = manifest_files(manifest_json, contents).map_err(to_js)?;
        self.inner.add_files(files);
        Ok(())
    }

    /// JSON array of the lowercase `gfx` file names the definitions reference.
    ///
    /// Call after the text files are in: the browser can then skip reading the
    /// art nothing points at, which is roughly a third of the emblems and most
    /// of the goods icons.
    #[wasm_bindgen(js_name = neededGfxNames)]
    pub fn needed_gfx_names(&mut self) -> Result<String, JsError> {
        let names = self
            .inner
            .needed_gfx_names()
            .map_err(|error| to_js(WasmError::from(error)))?;
        serde_json::to_string(&names).map_err(|error| to_js(WasmError::from(error)))
    }

    /// Encode everything absorbed so far. The builder is empty afterwards.
    pub fn finish(&mut self) -> Result<Vec<u8>, JsError> {
        let defs = std::mem::take(&mut self.inner)
            .finish()
            .map_err(|error| to_js(WasmError::from(error)))?;
        vic3_defs::encode_blob(&defs).map_err(|error| to_js(WasmError::from(error)))
    }
}

/// Canonical file/directory allowlist used by the browser's local folder walk.
#[wasm_bindgen]
pub fn classify_defs_path(path: &str, is_directory: bool) -> String {
    match vic3_defs::classify_defs_path(path, is_directory) {
        vic3_defs::DefsPathClass::Read => "read",
        vic3_defs::DefsPathClass::Skip => "skip",
        vic3_defs::DefsPathClass::Descend => "descend",
        vic3_defs::DefsPathClass::Prune => "prune",
    }
    .to_string()
}

/// Report what a definitions blob contains, so a UI can tell a full install
/// blob from the tiny demo fixture.
#[wasm_bindgen]
pub fn defs_summary(defs_blob: &[u8]) -> Result<String, JsError> {
    defs_summary_json(defs_blob).map_err(to_js)
}

/// Goods and extra icons as PNG data URLs, ready for an `img` tag.
///
/// Shape: `{ grain: url, goods: { grain: url }, extra: { "building:foo": url } }`.
/// Goods are nested and also flattened at the top level for older UI.
#[wasm_bindgen]
pub fn defs_icons(defs_blob: &[u8]) -> Result<String, JsError> {
    defs_icons_json(defs_blob).map_err(to_js)
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

/// Diagnose shortages from a save without retaining a worker session.
#[wasm_bindgen]
pub fn alerts(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, JsError> {
    alerts_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
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

#[derive(Serialize)]
struct LoadedAnalysisPayload<'a> {
    summary: SaveSummary,
    prices: &'a PricesResult,
}

/// Native/test entry for loading the worker-owned analysis session.
pub fn load_analysis_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let mut defs = vic3_defs::decode_blob(defs_blob)?;
    // The UI extracts icons separately. They are never consulted by World or
    // the solver, so do not retain their PNG bytes in the worker session too.
    defs.icons.clear();
    defs.extra_icons.clear();
    let solve_opts = parse_solve_opts(solve_opts_json)?;
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, solve_opts.clone());
    let json = serde_json::to_string(&LoadedAnalysisPayload {
        summary: SaveSummary::from(&save),
        prices: &prices,
    })?;
    LOADED_ANALYSIS.with(|loaded| {
        loaded.replace(Some(LoadedAnalysis {
            defs,
            world,
            solve_opts,
            prices,
            save,
        }));
    });
    Ok(json)
}

pub fn loaded_prices_json() -> Result<String, WasmError> {
    with_loaded_analysis(|loaded| Ok(serde_json::to_string(&loaded.prices)?))
}

pub fn loaded_military_json() -> Result<String, WasmError> {
    with_loaded_analysis(|loaded| {
        let player = played_country(&loaded.save).map(|(id, _)| id);
        Ok(serde_json::to_string(&military_snapshot(
            &loaded.save,
            player,
        ))?)
    })
}

pub fn loaded_what_if_json(what_if_opts_json: &str) -> Result<String, WasmError> {
    let delta: WhatIfOpts = serde_json::from_str(what_if_opts_json)?;
    with_loaded_analysis(|loaded| {
        let result = solve_what_if(
            &loaded.world,
            &loaded.defs,
            &delta,
            loaded.solve_opts.clone(),
        );
        Ok(serde_json::to_string(&result)?)
    })
}

pub fn loaded_apply_delta_json(delta_json: &str) -> Result<String, WasmError> {
    let delta: WorldDelta = serde_json::from_str(delta_json)?;
    with_loaded_analysis(|loaded| {
        let mut opts = loaded.solve_opts.clone();
        if !loaded.prices.relative.is_empty() {
            opts.warm_rel = Some(loaded.prices.relative.clone());
        }
        let result = solve_preview(&loaded.world, &loaded.defs, &delta, opts);
        Ok(serde_json::to_string(&result)?)
    })
}

pub fn loaded_gaps_json(goal: &str) -> Result<String, WasmError> {
    let goal = vic3_goals::parse(goal)?;
    with_loaded_analysis(|loaded| {
        let country = country_tag(&loaded.world)?;
        let state = PlanningState::from_world_with_prices(&loaded.world, country, &loaded.prices)?;
        let result = GapsResult {
            satisfied: vic3_goals::evaluate(&goal, &state),
            gaps: vic3_goals::gaps(&goal, &state),
            limitations: loaded.prices.limitations.clone(),
        };
        Ok(serde_json::to_string(&result)?)
    })
}

pub fn loaded_plan_json(plan_opts_json: &str) -> Result<String, WasmError> {
    let plan_opts: PlanOpts = serde_json::from_str(plan_opts_json)?;
    let goal = vic3_goals::parse(&plan_opts.goal)?;
    with_loaded_analysis(|loaded| {
        let country = country_tag(&loaded.world)?;
        let state = PlanningState::from_world_with_prices(&loaded.world, country, &loaded.prices)?;
        let economy = EconomyContext::new(
            loaded.world.clone(),
            loaded.defs.clone(),
            loaded.solve_opts.clone(),
        );
        let result = vic3_plan::plan_with_economy(
            state,
            goal,
            SimConfig::default(),
            economy,
            plan_opts.max_days,
            loaded.prices.residual,
            loaded.prices.limitations.clone(),
        )?;
        Ok(serde_json::to_string(&result)?)
    })
}

pub fn loaded_alerts_json() -> Result<String, WasmError> {
    with_loaded_analysis(|loaded| {
        let result = diagnose_alerts(&loaded.world, &loaded.defs, &loaded.prices);
        Ok(serde_json::to_string(&result)?)
    })
}

fn with_loaded_analysis<T>(
    run: impl FnOnce(&LoadedAnalysis) -> Result<T, WasmError>,
) -> Result<T, WasmError> {
    LOADED_ANALYSIS.with(|loaded| {
        let loaded = loaded.borrow();
        run(loaded.as_ref().ok_or(WasmError::NoLoadedAnalysis)?)
    })
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
    let world = World::from_save(&save, &defs);
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
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, solve_opts.clone());
    let country = country_tag(&world)?;
    drop(save);
    let state = PlanningState::from_world_with_prices(&world, country, &prices)?;
    let goal = vic3_goals::parse(&plan_opts.goal)?;
    let economy = EconomyContext::new(world, defs, solve_opts);
    let result = vic3_plan::plan_with_economy(
        state,
        goal,
        SimConfig::default(),
        economy,
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
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, opts);
    let country = country_tag(&world)?;
    drop(save);
    let state = PlanningState::from_world_with_prices(&world, country, &prices)?;
    let goal = vic3_goals::parse(goal)?;
    let result = GapsResult {
        satisfied: vic3_goals::evaluate(&goal, &state),
        gaps: vic3_goals::gaps(&goal, &state),
        limitations: prices.limitations,
    };
    Ok(serde_json::to_string(&result)?)
}

/// Native/test entry: same `AlertsResult` JSON as a future CLI `alerts` command.
pub fn alerts_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, WasmError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, opts);
    let result = diagnose_alerts(&world, &defs, &prices);
    Ok(serde_json::to_string(&result)?)
}

fn country_tag(world: &World) -> Result<&str, WasmError> {
    world.player_country_tag().ok_or(WasmError::NoCountry)
}

fn played_country(save: &Save) -> Option<(u32, &vic3_load::Country)> {
    save.previous_played.iter().find_map(|player| {
        let id = player.idtype?;
        save.country_manager
            .database
            .get(&id)
            .and_then(Option::as_ref)
            .map(|country| (id, country))
    })
}

const MILITARY_INCOMPLETE: &str = "Military IR incomplete; missing managers yield empty lists";

#[derive(Debug, Serialize)]
struct MilitarySnapshot {
    armies: Vec<FormationSnap>,
    navies: Vec<FormationSnap>,
    mobilization: Vec<MobilizationSnap>,
    limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FormationSnap {
    id: u32,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    country: Option<u32>,
    organization: Option<f64>,
    current_manpower: Option<f64>,
    units: Vec<UnitSnap>,
}

#[derive(Debug, Serialize)]
struct UnitSnap {
    id: Option<u32>,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    manpower: Option<f64>,
}

#[derive(Debug, Serialize)]
struct MobilizationSnap {
    id: u32,
    name: Option<String>,
    country: Option<u32>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn military_snapshot(save: &Save, player: Option<u32>) -> MilitarySnapshot {
    let mut armies = Vec::new();
    let mut navies = Vec::new();
    let mut parsed_formations = 0usize;

    let mut push_formation =
        |id: u32, formation: &vic3_load::MilitaryFormation, force_navy: Option<bool>| {
            parsed_formations += 1;
            if !matches_player(formation.country, player) {
                return;
            }
            let snap = FormationSnap {
                id,
                name: formation.name.clone(),
                kind: formation.kind.clone(),
                country: formation.country,
                organization: formation.organization,
                current_manpower: formation.current_manpower,
                units: formation
                    .units
                    .iter()
                    .map(|unit| UnitSnap {
                        id: unit.id,
                        name: unit.name.clone(),
                        kind: unit.kind.clone(),
                        manpower: unit.manpower,
                    })
                    .collect(),
            };
            let navy = force_navy.unwrap_or_else(|| is_navy(formation.kind.as_deref()));
            if navy {
                navies.push(snap);
            } else {
                armies.push(snap);
            }
        };

    for (id, formation) in save.formation_manager.iter_present() {
        push_formation(id, formation, None);
    }
    for (id, formation) in save.military_formations.iter_present() {
        push_formation(id, formation, None);
    }
    for (id, formation) in save.armies.iter_present() {
        push_formation(id, formation, Some(false));
    }
    for (id, formation) in save.navy_manager.iter_present() {
        push_formation(id, formation, Some(true));
    }

    let mut mobilization = Vec::new();
    let mut parsed_mobilization = 0usize;
    for (id, entry) in save.mobilization.iter_present() {
        parsed_mobilization += 1;
        if !matches_player(entry.country, player) {
            continue;
        }
        mobilization.push(MobilizationSnap {
            id,
            name: entry.name.clone(),
            country: entry.country,
            kind: entry.kind.clone(),
        });
    }

    let mut limitations = Vec::new();
    if parsed_formations == 0 && parsed_mobilization == 0 {
        limitations.push(MILITARY_INCOMPLETE.to_string());
    }

    MilitarySnapshot {
        armies,
        navies,
        mobilization,
        limitations,
    }
}

fn matches_player(country: Option<u32>, player: Option<u32>) -> bool {
    match (country, player) {
        (Some(country), Some(player)) => country == player,
        _ => true,
    }
}

fn is_navy(kind: Option<&str>) -> bool {
    kind.is_some_and(|kind| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "navy" | "fleet" | "flotilla" | "naval"
        )
    })
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
    let world = World::from_save(&save, &defs);
    Ok(solve(&world, &defs, opts))
}

fn parse_solve_opts(json: &str) -> Result<SolveOpts, WasmError> {
    let json = json.trim();
    if json.is_empty() {
        return Ok(SolveOpts::default());
    }
    Ok(serde_json::from_str(json)?)
}

#[derive(Debug, Deserialize)]
struct DefsFileEntry {
    path: String,
    offset: usize,
    length: usize,
}

/// Resolve a `{path, offset, length}` manifest against its payload.
fn manifest_files(
    manifest_json: &str,
    contents: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, WasmError> {
    let manifest: Vec<DefsFileEntry> = serde_json::from_str(manifest_json)?;
    manifest
        .into_iter()
        .map(|entry| {
            let end = entry.offset.checked_add(entry.length).ok_or_else(|| {
                WasmError::DefsManifest(format!("offset overflow for {}", entry.path))
            })?;
            let bytes = contents.get(entry.offset..end).ok_or_else(|| {
                WasmError::DefsManifest(format!(
                    "{} references {}..{} of {} bytes",
                    entry.path,
                    entry.offset,
                    end,
                    contents.len()
                ))
            })?;
            Ok((entry.path, bytes.to_vec()))
        })
        .collect()
}

/// Native/test entry for [`build_defs_blob`].
pub fn build_defs_blob_bytes(manifest_json: &str, contents: &[u8]) -> Result<Vec<u8>, WasmError> {
    let defs = vic3_defs::load_from_files(manifest_files(manifest_json, contents)?)?;
    Ok(vic3_defs::encode_blob(&defs)?)
}

/// Counts carried by a definitions blob.
#[derive(Debug, Serialize)]
struct DefsSummary {
    blob_version: u32,
    goods: usize,
    labels: usize,
    icons: usize,
    production_methods: usize,
    pop_needs: usize,
    buy_packages: usize,
    price_range: f64,
}

/// Native/test entry for [`defs_summary`].
pub fn defs_summary_json(defs_blob: &[u8]) -> Result<String, WasmError> {
    let defs = vic3_defs::decode_blob(defs_blob)?;
    Ok(serde_json::to_string(&DefsSummary {
        blob_version: vic3_defs::BLOB_VERSION,
        goods: defs.goods.len(),
        labels: defs.labels.len(),
        icons: defs.icons.len() + defs.extra_icons.len(),
        production_methods: defs.production_methods.len(),
        pop_needs: defs.pop_needs.len(),
        buy_packages: defs.buy_packages.len(),
        price_range: defs.price_range,
    })?)
}

/// Native/test entry for [`defs_icons`].
pub fn defs_icons_json(defs_blob: &[u8]) -> Result<String, WasmError> {
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let png_url = |png: &[u8]| format!("data:image/png;base64,{}", base64(png));
    let mut urls = serde_json::Map::new();
    let mut goods = serde_json::Map::new();
    for (id, png) in &defs.icons {
        let url = serde_json::Value::String(png_url(png));
        goods.insert(id.clone(), url.clone());
        urls.insert(id.clone(), url);
    }
    let extra = defs
        .extra_icons
        .iter()
        .map(|(id, png)| (id.clone(), serde_json::Value::String(png_url(png))))
        .collect();
    urls.insert("goods".into(), serde_json::Value::Object(goods));
    urls.insert("extra".into(), serde_json::Value::Object(extra));
    Ok(serde_json::Value::Object(urls).to_string())
}

/// Standard base64, used only to hand PNG bytes to an `img` element.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = chunk
            .iter()
            .chain(std::iter::repeat(&0))
            .take(3)
            .fold(0u32, |packed, byte| packed << 8 | u32::from(*byte));
        let digit = |shift: u32| ALPHABET[(triple >> shift & 63) as usize] as char;
        out.push(digit(18));
        out.push(digit(12));
        out.push(if chunk.len() > 1 { digit(6) } else { '=' });
        out.push(if chunk.len() > 2 { digit(0) } else { '=' });
    }
    out
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
    country_id: Option<u32>,
    market_id: Option<u32>,
    date: Option<String>,
    version: String,
    counts: SaveCounts,
    buildings: Vec<String>,
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
        let selected_country = played_country(save).or_else(|| save.countries().next());
        let tag = selected_country.map(|(_, country)| country.definition.clone());
        let country_id = selected_country.map(|(id, _)| id);
        let market_id = selected_country.and_then(|(_, country)| country.market);
        let date = save.meta_data.game_date.map(|d| d.game_fmt().to_string());
        Self {
            tag,
            country_id,
            market_id,
            date,
            version: save.meta_data.version.clone(),
            buildings: save
                .building_manager
                .iter_present()
                .filter_map(|(_, building)| {
                    (!building.building.is_empty()).then_some(building.building.clone())
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
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

    #[test]
    fn builds_defs_blob_from_browser_manifest() {
        let goods = br#"grain = { cost = 20 }"#;
        let manifest = format!(
            r#"[{{"path":"Victoria 3/game/common/goods/goods.txt","offset":0,"length":{}}}]"#,
            goods.len()
        );
        let blob = build_defs_blob_bytes(&manifest, goods).expect("browser defs blob");
        let defs = vic3_defs::decode_blob(&blob).expect("decode browser blob");
        assert_eq!(defs.base_price("grain"), Some(20.0));
    }

    #[test]
    fn defs_summary_counts_blob_contents() {
        let json = defs_summary_json(&defs_blob()).expect("defs summary");
        let v: Value = serde_json::from_str(&json).expect("summary json");
        assert_eq!(v["blob_version"], vic3_defs::BLOB_VERSION);
        assert_eq!(v["goods"], 3);
        assert_eq!(v["labels"], 8);
        assert_eq!(v["icons"], 2);
        assert!(v["price_range"].as_f64().is_some_and(|range| range > 0.0));
    }

    #[test]
    fn defs_icons_are_png_data_urls_keyed_by_good() {
        let json = defs_icons_json(&defs_blob()).expect("defs icons");
        let v: Value = serde_json::from_str(&json).expect("icons json");
        let grain = v["grain"].as_str().expect("grain icon");
        assert!(grain.starts_with("data:image/png;base64,iVBOR"), "{grain}");
        let nested = v["goods"]["grain"].as_str().expect("nested goods grain");
        assert_eq!(nested, grain);
        let building = v["extra"]["building:building_rye_farm"]
            .as_str()
            .expect("building extra icon");
        assert!(
            building.starts_with("data:image/png;base64,iVBOR"),
            "{building}"
        );
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
    fn defs_path_classifier_exposes_rust_allowlist() {
        assert_eq!(
            classify_defs_path("game/common/goods/00_goods.txt", false),
            "read"
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/goods_icons/grain.dds", false),
            "read"
        );
        assert_eq!(
            classify_defs_path(
                "game/gfx/interface/icons/building_icons/building_rye_farm.dds",
                false
            ),
            "read"
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/country_icons", true),
            "prune"
        );
        assert_eq!(classify_defs_path("game/gfx/models", true), "prune");
    }

    #[test]
    fn parse_save_plaintext_summary() {
        let json = parse_save_json(&load_fixture(), None).expect("parse plaintext");
        let v: Value = serde_json::from_str(&json).expect("summary json");
        assert_eq!(v["tag"], "GER");
        assert_eq!(v["country_id"], 16777216);
        assert_eq!(v["market_id"], 1);
        assert_eq!(v["date"], "1836.1.1");
        assert_eq!(v["version"], "1.9.0");
        assert_eq!(v["counts"]["countries"], 1);
        assert_eq!(v["counts"]["states"], 1);
        assert_eq!(v["counts"]["buildings"], 1);
        assert_eq!(v["counts"]["pops"], 1);
        assert_eq!(v["counts"]["markets"], 1);
        assert_eq!(v["counts"]["trade_routes"], 1);
        assert_eq!(v["buildings"][0], "building_rye_farm");
    }

    #[test]
    fn save_summary_falls_back_to_country_matching_tag() {
        let mut save = load_save(&load_fixture(), None).expect("parse plaintext");
        save.previous_played[0].idtype = None;

        let v = serde_json::to_value(SaveSummary::from(&save)).expect("summary json");
        assert_eq!(v["country_id"], 16777216);
        assert_eq!(v["market_id"], 1);
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
    fn loaded_analysis_keeps_baseline_world_and_prices() {
        clear_analysis();
        assert!(matches!(
            loaded_prices_json(),
            Err(WasmError::NoLoadedAnalysis)
        ));

        let json =
            load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let payload: Value = serde_json::from_str(&json).expect("loaded payload");
        assert_eq!(payload["summary"]["tag"], "GER");
        assert!(payload["prices"]["goods"].is_array());

        let baseline: PricesResult =
            serde_json::from_str(&loaded_prices_json().expect("cached prices"))
                .expect("PricesResult");
        assert!(!baseline.goods.is_empty());
        let changed: PricesResult = serde_json::from_str(
            &loaded_what_if_json(r#"{"building":"building_rye_farm","extra_levels":5}"#)
                .expect("cached what-if"),
        )
        .expect("PricesResult");
        assert!(changed.residual.is_finite());

        clear_analysis();
    }

    #[test]
    fn loaded_apply_delta_does_not_change_subsequent_loaded_prices() {
        clear_analysis();
        load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let baseline = loaded_prices_json().expect("cached prices");
        let previewed = loaded_apply_delta_json(
            r#"{"extra_levels":[{"building":"building_rye_farm","extra_levels":5}]}"#,
        )
        .expect("preview delta");
        let after = loaded_prices_json().expect("prices after preview");
        assert_eq!(after, baseline, "preview must not commit loaded prices");
        let previewed_result: PricesResult =
            serde_json::from_str(&previewed).expect("preview PricesResult");
        assert!(previewed_result.residual.is_finite());
        clear_analysis();
    }

    #[test]
    fn loaded_military_json_after_load_has_army_and_navy_arrays() {
        clear_analysis();
        load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let json = loaded_military_json().expect("military snapshot");
        let v: Value = serde_json::from_str(&json).expect("military json");
        assert!(v["armies"].is_array(), "{v}");
        assert!(v["navies"].is_array(), "{v}");
        assert!(v["mobilization"].is_array(), "{v}");
        let limitations = v["limitations"].as_array().expect("limitations array");
        assert!(
            limitations.iter().any(|item| item.as_str()
                == Some("Military IR incomplete; missing managers yield empty lists")),
            "{v}"
        );
        clear_analysis();
        assert!(matches!(
            loaded_military_json(),
            Err(WasmError::NoLoadedAnalysis)
        ));
    }

    #[test]
    fn loaded_alerts_after_load_analysis() {
        clear_analysis();
        load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let json = loaded_alerts_json().expect("loaded alerts");
        let result: vic3_prices::AlertsResult = serde_json::from_str(&json).expect("AlertsResult");
        assert!(json.contains("\"alerts\""));
        assert!(
            result.limitations.iter().any(|line| !line.is_empty())
                || json.contains("\"limitations\"")
        );
        clear_analysis();
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

    #[test]
    #[ignore = "set VIC3_SAVE, VIC3_TOKENS, and VIC3_GAME or VIC3_DEFS"]
    fn live_load_analysis_session() {
        use std::time::Instant;

        let save = std::fs::read(std::env::var("VIC3_SAVE").expect("VIC3_SAVE")).expect("save");
        let tokens = std::env::var("VIC3_TOKENS")
            .ok()
            .map(|path| std::fs::read(path).expect("tokens"));
        let blob = if let Ok(path) = std::env::var("VIC3_DEFS") {
            std::fs::read(path).expect("defs blob")
        } else {
            let defs =
                load_from_path(std::env::var("VIC3_GAME").expect("VIC3_GAME")).expect("game defs");
            encode_blob(&defs).expect("encode blob")
        };
        clear_analysis();
        let started = Instant::now();
        let json = load_analysis_json(&save, tokens.as_deref(), &blob, "{}").expect("load");
        eprintln!("load_analysis {:?}", started.elapsed());
        let payload: Value = serde_json::from_str(&json).expect("payload");
        assert!(payload["summary"]["tag"].is_string());
        let started = Instant::now();
        loaded_gaps_json("research(tech=nitroglycerin)").expect("gaps");
        eprintln!("loaded_gaps {:?}", started.elapsed());
        clear_analysis();
    }
}
