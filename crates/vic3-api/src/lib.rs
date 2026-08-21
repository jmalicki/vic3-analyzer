//! Transport-free analysis API: bytes or paths in, JSON out.
//!
//! # Why this crate exists
//!
//! CLI, wasm, Tauri, SQL (`use_save` / `latest.*`), and MCP all need the **same**
//! analysis results. Putting load + solve + serialize here keeps option and
//! result JSON identical across hosts. Transport stays outside:
//!
//! | Host | Input shape | This crate |
//! | --- | --- | --- |
//! | `vic3-wasm` | `Vec<u8>` + JSON option strings | `*_json` / `loaded_*_json` |
//! | Tauri / MCP | paths or catalog stubs | `*_from_paths` / session |
//! | `vic3-cli` | clap → paths (clap only in that crate) | same shapes; CLI may call core directly |
//!
//! No `wasm_bindgen`, clap, or host filesystem policy beyond the optional path
//! helpers. Pipeline: save → defs → prices → plan — see [`docs/architecture.md`](../../../docs/architecture.md).
//!
//! # Two call styles
//!
//! 1. **One-shot** — `prices_json`, `what_if_json`, `gaps_json`, `plan_json`,
//!    `alerts_json`, … take save/defs bytes (or `*_from_paths`) and return JSON.
//!    Does not install a session.
//! 2. **Session** — [`load_analysis_json`] (or [`load_analysis_snapshot`] with
//!    `install = true`) stores defs, world, baseline prices, and save IR in a
//!    process-local cell. Follow-ups: [`loaded_prices_json`],
//!    [`loaded_what_if_json`], [`loaded_gaps_json`], [`loaded_plan_json`],
//!    [`loaded_alerts_json`], [`loaded_apply_delta_json`],
//!    [`loaded_optimize_pms_json`], [`loaded_military_json`],
//!    [`loaded_constructions_json`], [`loaded_production_methods_json`].
//!    [`clear_analysis`] drops the session.
//!
//! Wasm hosts the session in the analysis worker (one at a time). Native hosts
//! share the same model. [`load_analysis_snapshot`] with `install = false` builds
//! an owned snapshot without mutating the active session (SQL `latest.*`).
//!
//! # Contracts
//!
//! - Inner option structs ([`vic3_prices::SolveOpts`], [`vic3_prices::WhatIfOpts`],
//!   [`vic3_plan::PlanOpts`], …) have **no** `PathBuf`; paths appear only on
//!   path helpers and clap wrappers.
//! - Empty / `{}` / whitespace `solve_opts_json` → [`SolveOpts::default`].
//! - Most analysis exports return `Result<String, ApiError>` (JSON text).
//!   [`export_save_bytes`] returns patched plaintext bytes.
//! - Preview APIs ([`loaded_apply_delta_json`], [`loaded_optimize_pms_json`]) do
//!   **not** replace the loaded world or baseline prices.
//! - Errors: [`ApiError`] (load, defs, JSON, goals, plan, [`ApiError::NoLoadedAnalysis`], …).
//!
//! Usage overview: [`docs/usage.md`](../../../docs/usage.md). Result schemas:
//! [`docs/json-schema.md`](../../../docs/json-schema.md).

mod error;

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;
use vic3_goals::Atom;
use vic3_load::{empty_tokens, load_slice, load_tokens_slice, Save, SavePatch};
use vic3_plan::PlanOpts;
use vic3_prices::{
    alerts as diagnose_alerts, optimize_pms, preview as solve_preview, solve,
    what_if as solve_what_if, OptimizePmsOpts, PricesResult, SolveOpts, WhatIfOpts, World,
    WorldDelta,
};
use vic3_sim::{EconomyContext, SimConfig};
use vic3_world::PlanningState;
use vic3save::PdsDate;

pub use error::ApiError;

struct LoadedAnalysis {
    defs: vic3_defs::GameDefs,
    world: World,
    solve_opts: SolveOpts,
    prices: PricesResult,
    save: Save,
}

thread_local! {
    /// Process-local analysis session (one at a time). Wasm hosts this in the
    /// analysis worker; native callers share the same model.
    static LOADED_ANALYSIS: RefCell<Option<LoadedAnalysis>> = const { RefCell::new(None) };
}

/// Read a file into memory for path-based loaders.
///
/// # Errors
///
/// [`ApiError::Io`] when the path cannot be read.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, ApiError> {
    std::fs::read(path).map_err(|source| ApiError::io(path, source))
}

/// Load optional token-map bytes from a path (`None` → no tokens).
///
/// # Errors
///
/// [`ApiError::Io`] when `tokens` is `Some` and the file cannot be read.
pub fn read_tokens(tokens: Option<&Path>) -> Result<Option<Vec<u8>>, ApiError> {
    match tokens {
        None => Ok(None),
        Some(path) => Ok(Some(read_bytes(path)?)),
    }
}

/// Load a save from paths (plaintext needs no tokens).
///
/// # Errors
///
/// [`ApiError::Io`] on read failure; [`ApiError::Load`] on parse / missing tokens.
pub fn load_save_from_path(save: &Path, tokens: Option<&Path>) -> Result<Save, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    load_save(&save_bytes, tokens_bytes.as_deref())
}

/// Read a postcard definitions blob from disk.
///
/// # Errors
///
/// [`ApiError::Io`] when the path cannot be read.
pub fn read_defs_blob(path: &Path) -> Result<Vec<u8>, ApiError> {
    read_bytes(path)
}

/// Build a postcard definitions blob from a Victoria 3 install / fixture tree.
///
/// # Errors
///
/// [`ApiError::Defs`] when the tree cannot be loaded or encoded.
pub fn defs_blob_from_game(game: &Path) -> Result<Vec<u8>, ApiError> {
    let defs = vic3_defs::load_from_path(game)?;
    Ok(vic3_defs::encode_blob(&defs)?)
}

/// Cached defs postcard filename under the platform app-data directory.
///
/// Shared by the Tauri companion and `vic3-analyzer mcp` so Settings and MCP
/// resolve the same blob path for a given install.
pub const DEFS_CACHE_NAME: &str = "defs.postcard";

/// Resolve or build a defs postcard for desktop hosts (GUI + MCP).
///
/// Order: explicit `defs_blob` file → existing `app_data/defs.postcard` → build
/// from `game_dir` and write the cache. Callers should validate `game_dir` with
/// [`vic3_catalog::is_valid_game_dir`] before relying on a live install path.
///
/// First-time builds from a full Steam install decode many DDS icons and can
/// take minutes; GUI Settings and MCP share this cache so the cost is paid once
/// per app-data directory.
pub fn ensure_defs_blob(
    defs_blob: Option<&Path>,
    game_dir: Option<&Path>,
    app_data: &Path,
) -> Result<std::path::PathBuf, ApiError> {
    if let Some(blob) = defs_blob {
        if blob.is_file() {
            return Ok(blob.to_path_buf());
        }
        return Err(ApiError::Config(format!(
            "defs_blob not found: {}",
            blob.display()
        )));
    }
    let cache = app_data.join(DEFS_CACHE_NAME);
    if cache.is_file() {
        return Ok(cache);
    }
    let game = game_dir.ok_or_else(|| {
        ApiError::Config(
            "no game_dir or defs_blob configured — set paths in Settings or enable auto-detect"
                .into(),
        )
    })?;
    if !game.join("common").is_dir() {
        return Err(ApiError::Config(format!(
            "game_dir is not a valid Victoria 3 game tree: {}",
            game.display()
        )));
    }
    let bytes = defs_blob_from_game(game)?;
    std::fs::write(&cache, &bytes).map_err(|source| ApiError::io(&cache, source))?;
    Ok(cache)
}

/// Path convenience for [`load_analysis_json`].
///
/// # Errors
///
/// Propagates IO, load, defs, and JSON errors from the bytes path.
pub fn load_analysis_from_paths(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    load_analysis_json(&save_bytes, tokens_bytes.as_deref(), &defs, solve_opts_json)
}

/// Path convenience for [`prices_json`].
pub fn prices_from_paths(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    prices_json(&save_bytes, tokens_bytes.as_deref(), &defs, solve_opts_json)
}

/// Path convenience for [`what_if_json`].
pub fn what_if_from_paths(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
    what_if_opts_json: &str,
) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    what_if_json(
        &save_bytes,
        tokens_bytes.as_deref(),
        &defs,
        solve_opts_json,
        what_if_opts_json,
    )
}

/// Path convenience for [`plan_json`].
pub fn plan_from_paths(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
    plan_opts_json: &str,
) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    plan_json(
        &save_bytes,
        tokens_bytes.as_deref(),
        &defs,
        solve_opts_json,
        plan_opts_json,
    )
}

/// Path convenience for [`gaps_json`].
pub fn gaps_from_paths(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
    goal: &str,
) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    gaps_json(
        &save_bytes,
        tokens_bytes.as_deref(),
        &defs,
        solve_opts_json,
        goal,
    )
}

/// Path convenience for [`alerts_json`].
pub fn alerts_from_paths(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    alerts_json(&save_bytes, tokens_bytes.as_deref(), &defs, solve_opts_json)
}

/// Path convenience for [`parse_save_json`].
pub fn parse_save_from_path(save: &Path, tokens: Option<&Path>) -> Result<String, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    parse_save_json(&save_bytes, tokens_bytes.as_deref())
}

/// Clear the process-local analysis session.
///
/// Idempotent. After this, every `loaded_*` call returns [`ApiError::NoLoadedAnalysis`].
pub fn clear_analysis() {
    LOADED_ANALYSIS.with(|loaded| {
        loaded.borrow_mut().take();
    });
}

/// Parse a save into a compact summary JSON (tag, date, counts, building types).
///
/// Same payload as the wasm `parse_save` export. Does not install a session.
///
/// # Arguments
///
/// * `save_bytes` — raw `.v3` (plaintext or binary).
/// * `tokens_bytes` — Paradox token map; omit / empty for plaintext.
///
/// # Errors
///
/// [`ApiError::Load`] (including `MissingTokens` for binary without tokens);
/// [`ApiError::Json`] on serialize failure.
pub fn parse_save_json(save_bytes: &[u8], tokens_bytes: Option<&[u8]>) -> Result<String, ApiError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    Ok(serde_json::to_string(&SaveSummary::from(&save))?)
}

#[derive(Serialize)]
struct LoadedAnalysisPayload<'a> {
    summary: SaveSummary,
    prices: &'a PricesResult,
}

/// Owned analysis snapshot after load + baseline price solve.
///
/// Used by hosts that need Rust handles (e.g. `vic3-sql` `use_save`) rather
/// than only JSON. Prefer [`load_analysis_snapshot`] over re-parsing JSON.
#[derive(Debug, Clone)]
pub struct AnalysisSnapshot {
    pub defs: vic3_defs::GameDefs,
    pub world: World,
    pub prices: PricesResult,
    /// Played country tag when known.
    pub tag: Option<String>,
    /// In-game date string when known.
    pub date: Option<String>,
}

/// Shared load + solve path for JSON install and owned snapshots.
fn build_loaded_analysis(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<(LoadedAnalysis, SaveSummary), ApiError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let mut defs = vic3_defs::decode_blob(defs_blob)?;
    // The UI extracts icons separately. They are never consulted by World or
    // the solver, so do not retain their PNG bytes in the worker session too.
    defs.icons.clear();
    defs.extra_icons.clear();
    let solve_opts = parse_solve_opts(solve_opts_json)?;
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, solve_opts.clone());
    let summary = SaveSummary::from(&save);
    Ok((
        LoadedAnalysis {
            defs,
            world,
            solve_opts,
            prices,
            save,
        },
        summary,
    ))
}

/// Load + solve into an [`AnalysisSnapshot`].
///
/// When `install` is true, also replaces the process-local analysis session
/// (same as [`load_analysis_json`]). Pass `false` for read-side caches such as
/// SQL `latest.*` so the active session is not mutated.
///
/// # Arguments
///
/// * `defs_blob` — postcard blob from [`vic3_defs::encode_blob`] (or
///   [`build_defs_blob_bytes`] / CLI `defs export`).
/// * `solve_opts_json` — [`SolveOpts`] JSON; empty / `{}` uses defaults.
/// * `install` — bind the process-local session when true.
///
/// # Errors
///
/// Load, defs decode, invalid solve opts JSON, or serialize failures.
pub fn load_analysis_snapshot(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    install: bool,
) -> Result<AnalysisSnapshot, ApiError> {
    let (loaded, summary) =
        build_loaded_analysis(save_bytes, tokens_bytes, defs_blob, solve_opts_json)?;
    let snap = AnalysisSnapshot {
        defs: loaded.defs.clone(),
        world: loaded.world.clone(),
        prices: loaded.prices.clone(),
        tag: summary.tag,
        date: summary.date,
    };
    if install {
        LOADED_ANALYSIS.with(|cell| {
            cell.replace(Some(loaded));
        });
    }
    Ok(snap)
}

/// Path convenience for [`load_analysis_snapshot`].
///
/// Same `install` contract: `true` for session bind (`use_save`), `false` for
/// read-side caches (`latest.*`).
pub fn load_analysis_snapshot_from_path(
    save: &Path,
    tokens: Option<&Path>,
    defs_blob: &Path,
    solve_opts_json: &str,
    install: bool,
) -> Result<AnalysisSnapshot, ApiError> {
    let save_bytes = read_bytes(save)?;
    let tokens_bytes = read_tokens(tokens)?;
    let defs = read_defs_blob(defs_blob)?;
    load_analysis_snapshot(
        &save_bytes,
        tokens_bytes.as_deref(),
        &defs,
        solve_opts_json,
        install,
    )
}

/// Load one analysis session and solve its baseline prices.
///
/// Always installs the process-local session (same as
/// [`load_analysis_snapshot`] with `install = true`). Returns JSON
/// `{ summary, prices }` where `prices` is a full [`PricesResult`].
/// Icon PNG bytes are stripped from the retained defs (UI loads icons separately).
///
/// # Errors
///
/// Load, defs, invalid `solve_opts_json`, or JSON serialize failures.
pub fn load_analysis_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, ApiError> {
    let (loaded, summary) =
        build_loaded_analysis(save_bytes, tokens_bytes, defs_blob, solve_opts_json)?;
    let json = serde_json::to_string(&LoadedAnalysisPayload {
        summary,
        prices: &loaded.prices,
    })?;
    LOADED_ANALYSIS.with(|cell| {
        cell.replace(Some(loaded));
    });
    Ok(json)
}

/// Baseline [`PricesResult`] JSON from the loaded session.
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`] if nothing is loaded; JSON serialize failures.
pub fn loaded_prices_json() -> Result<String, ApiError> {
    with_loaded_analysis(|loaded| Ok(serde_json::to_string(&loaded.prices)?))
}

/// Conservative military snapshot JSON for the played country.
///
/// Incomplete IR yields empty lists plus a limitations string.
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`] if nothing is loaded.
pub fn loaded_military_json() -> Result<String, ApiError> {
    with_loaded_analysis(|loaded| {
        let player = played_country(&loaded.save).map(|(id, _)| id);
        Ok(serde_json::to_string(&military_snapshot(
            &loaded.save,
            player,
        ))?)
    })
}

/// Private + government construction queues for the played country.
///
/// Rows come from [`vic3_prices::World::constructions`] (same projection as SQL).
/// Empty queues are valid (no limitations string).
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`] if nothing is loaded.
pub fn loaded_constructions_json() -> Result<String, ApiError> {
    with_loaded_analysis(|loaded| {
        let player = played_country(&loaded.save).map(|(id, _)| id);
        Ok(serde_json::to_string(&constructions_snapshot(
            &loaded.world,
            &loaded.defs,
            player,
        ))?)
    })
}

/// Patch plaintext `.v3` bytes with a [`SavePatch`]; returns a new buffer.
///
/// Does not touch the analysis session. Ironman / binary envelopes are rejected.
///
/// # Errors
///
/// Invalid `delta_json`, or [`ApiError::Export`] (binary / patch failure).
pub fn export_save_bytes(original_bytes: &[u8], delta_json: &str) -> Result<Vec<u8>, ApiError> {
    let patch: SavePatch = serde_json::from_str(delta_json)?;
    Ok(vic3_load::export_save(original_bytes, &patch)?)
}

/// What-if re-solve on the loaded world ([`WhatIfOpts`] JSON → [`PricesResult`]).
///
/// Does not replace the loaded baseline.
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`], invalid JSON.
pub fn loaded_what_if_json(what_if_opts_json: &str) -> Result<String, ApiError> {
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

/// Preview a [`WorldDelta`] on a clone of the loaded world (warm-started when possible).
///
/// Does not replace the loaded world or baseline prices, and does not write a save.
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`], invalid delta JSON.
pub fn loaded_apply_delta_json(delta_json: &str) -> Result<String, ApiError> {
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

/// Suggest production-method changes (`{"axis":"income"|"productivity"|"sol"}`).
///
/// Does not replace the loaded world or write a save.
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`], invalid axis JSON.
pub fn loaded_optimize_pms_json(axis_json: &str) -> Result<String, ApiError> {
    let opts: OptimizePmsOpts = serde_json::from_str(axis_json)?;
    with_loaded_analysis(|loaded| {
        let result = optimize_pms(
            &loaded.world,
            &loaded.defs,
            &loaded.prices,
            loaded.solve_opts.clone(),
            opts.axis,
        );
        Ok(serde_json::to_string(&result)?)
    })
}

/// Evaluate goal gaps against the loaded session (`GapsResult` JSON).
///
/// Shape: `{ satisfied, gaps, limitations }` — same as CLI `gaps --json`.
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`], goal parse, [`ApiError::NoCountry`], world projection.
pub fn loaded_gaps_json(goal: &str) -> Result<String, ApiError> {
    let goal = vic3_goals::parse(goal)?;
    with_loaded_analysis(|loaded| {
        let country = country_tag(&loaded.world)?;
        let state = PlanningState::from_world_with_prices(&loaded.world, country, &loaded.prices)?;
        let mut limitations = loaded.prices.limitations.clone();
        if goal.has_army_atom() {
            state.push_army_power_limitation(&mut limitations);
        }
        let result = GapsResult {
            satisfied: vic3_goals::evaluate(&goal, &state),
            gaps: vic3_goals::gaps(&goal, &state),
            limitations,
        };
        Ok(serde_json::to_string(&result)?)
    })
}

/// Plan from the loaded session ([`PlanOpts`] JSON → [`vic3_plan::PlanResult`]).
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`], invalid opts / goal, [`ApiError::NoCountry`],
/// world projection, or [`ApiError::Plan`].
pub fn loaded_plan_json(plan_opts_json: &str) -> Result<String, ApiError> {
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

/// Shortage alerts from the loaded session ([`vic3_prices::AlertsResult`] JSON).
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`].
pub fn loaded_alerts_json() -> Result<String, ApiError> {
    with_loaded_analysis(|loaded| {
        let result = diagnose_alerts(&loaded.world, &loaded.defs, &loaded.prices);
        Ok(serde_json::to_string(&result)?)
    })
}

#[derive(Serialize)]
struct ProductionMethodJson {
    id: String,
    inputs: Vec<PmFlowJson>,
    outputs: Vec<PmFlowJson>,
}

#[derive(Serialize)]
struct PmFlowJson {
    good: String,
    qty: f64,
}

/// Production-method recipes from loaded defs (`[{id, inputs, outputs}]`).
///
/// # Errors
///
/// [`ApiError::NoLoadedAnalysis`].
pub fn loaded_production_methods_json() -> Result<String, ApiError> {
    with_loaded_analysis(|loaded| {
        let methods: Vec<ProductionMethodJson> = loaded
            .defs
            .production_methods
            .values()
            .map(|pm| ProductionMethodJson {
                id: pm.id.clone(),
                inputs: pm
                    .inputs
                    .iter()
                    .filter_map(|(idx, qty)| {
                        loaded.defs.good_by_index(*idx).map(|good| PmFlowJson {
                            good: good.to_string(),
                            qty: *qty,
                        })
                    })
                    .collect(),
                outputs: pm
                    .outputs
                    .iter()
                    .filter_map(|(idx, qty)| {
                        loaded.defs.good_by_index(*idx).map(|good| PmFlowJson {
                            good: good.to_string(),
                            qty: *qty,
                        })
                    })
                    .collect(),
            })
            .collect();
        Ok(serde_json::to_string(&methods)?)
    })
}

fn with_loaded_analysis<T>(
    run: impl FnOnce(&LoadedAnalysis) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    LOADED_ANALYSIS.with(|loaded| {
        let loaded = loaded.borrow();
        run(loaded.as_ref().ok_or(ApiError::NoLoadedAnalysis)?)
    })
}

/// Solve market prices ([`PricesResult`] JSON). One-shot; does not install a session.
///
/// # Arguments
///
/// * `solve_opts_json` — [`SolveOpts`]; empty / `{}` → defaults.
///
/// # Errors
///
/// Load, defs, invalid opts JSON, or serialize failures.
pub fn prices_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, ApiError> {
    let result = run_prices(save_bytes, tokens_bytes, defs_blob, solve_opts_json)?;
    Ok(serde_json::to_string(&result)?)
}

/// Apply a what-if building-level delta and re-solve. One-shot.
///
/// `what_if_opts_json` is [`WhatIfOpts`] (`building`, `extra_levels`).
///
/// # Errors
///
/// Load, defs, invalid JSON, or serialize failures.
pub fn what_if_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    what_if_opts_json: &str,
) -> Result<String, ApiError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let delta: WhatIfOpts = serde_json::from_str(what_if_opts_json)?;
    let world = World::from_save(&save, &defs);
    let result = solve_what_if(&world, &defs, &delta, opts);
    Ok(serde_json::to_string(&result)?)
}

/// Find a shortest goal-relevant plan ([`vic3_plan::PlanResult`] JSON). One-shot.
///
/// `plan_opts_json` is [`PlanOpts`] (`goal`, `max_days`, optional `label`).
///
/// # Errors
///
/// Load, defs, goal/plan failures, [`ApiError::NoCountry`], or serialize failures.
pub fn plan_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    plan_opts_json: &str,
) -> Result<String, ApiError> {
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
    let economy = EconomyContext::new(world, defs, solve_opts.clone());
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

/// Evaluate a goal and return unsatisfied atoms (`GapsResult` JSON). One-shot.
///
/// # Errors
///
/// Load, defs, goal parse, [`ApiError::NoCountry`], world projection, or serialize failures.
pub fn gaps_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    goal: &str,
) -> Result<String, ApiError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, opts);
    let country = country_tag(&world)?;
    drop(save);
    let state = PlanningState::from_world_with_prices(&world, country, &prices)?;
    let goal = vic3_goals::parse(goal)?;
    let mut limitations = prices.limitations;
    if goal.has_army_atom() {
        state.push_army_power_limitation(&mut limitations);
    }
    let result = GapsResult {
        satisfied: vic3_goals::evaluate(&goal, &state),
        gaps: vic3_goals::gaps(&goal, &state),
        limitations,
    };
    Ok(serde_json::to_string(&result)?)
}

/// Diagnose shortages ([`vic3_prices::AlertsResult`] JSON). One-shot.
///
/// # Errors
///
/// Load, defs, invalid opts JSON, or serialize failures.
pub fn alerts_json(
    save_bytes: &[u8],
    tokens_bytes: Option<&[u8]>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, ApiError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let world = World::from_save(&save, &defs);
    let prices = solve(&world, &defs, opts);
    let result = diagnose_alerts(&world, &defs, &prices);
    Ok(serde_json::to_string(&result)?)
}

fn country_tag(world: &World) -> Result<&str, ApiError> {
    world.player_country_tag().ok_or(ApiError::NoCountry)
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

/// Build-queue JSON for the Buildings → Queues UI (government / private lists).
#[derive(Debug, Serialize)]
struct ConstructionsSnapshot {
    private: Vec<ConstructionSnap>,
    government: Vec<ConstructionSnap>,
}

#[derive(Debug, Serialize)]
struct ConstructionSnap {
    id: u32,
    queue: &'static str,
    country_id: Option<u32>,
    state_id: Option<u32>,
    /// Localized state region label when known.
    state_name: Option<String>,
    building: String,
    building_name: Option<String>,
    remaining: Option<f64>,
}

fn constructions_snapshot(
    world: &vic3_prices::World,
    defs: &vic3_defs::GameDefs,
    player: Option<u32>,
) -> ConstructionsSnapshot {
    // Player-scoped contract: no resolved player → empty queues (do not leak
    // foreign/unowned rows via matches_player's permissive military fallback).
    let Some(player_id) = player else {
        return ConstructionsSnapshot {
            private: Vec::new(),
            government: Vec::new(),
        };
    };
    let state_name = |state_id: Option<u32>| -> Option<String> {
        let id = state_id?;
        let state = world.states.iter().find(|s| s.id == id)?;
        state.region.clone().map(|region| {
            defs.labels
                .get(&region)
                .cloned()
                .unwrap_or_else(|| humanize_region(&region))
        })
    };
    let mut private = Vec::new();
    let mut government = Vec::new();
    for row in &world.constructions {
        if row.country_id != Some(player_id) {
            continue;
        }
        let snap = ConstructionSnap {
            id: row.id,
            queue: row.queue.as_str(),
            country_id: row.country_id,
            state_id: row.state_id,
            state_name: state_name(row.state_id),
            building: row.building.clone(),
            building_name: defs.labels.get(&row.building).cloned(),
            remaining: row.remaining,
        };
        match row.queue {
            vic3_prices::ConstructionQueueKind::Private => private.push(snap),
            vic3_prices::ConstructionQueueKind::Government => government.push(snap),
        }
    }
    ConstructionsSnapshot {
        private,
        government,
    }
}

fn humanize_region(region_id: &str) -> String {
    let trimmed = region_id
        .strip_prefix("STATE_")
        .or_else(|| region_id.strip_prefix("state_"))
        .unwrap_or(region_id);
    trimmed
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
) -> Result<PricesResult, ApiError> {
    let save = load_save(save_bytes, tokens_bytes)?;
    let defs = vic3_defs::decode_blob(defs_blob)?;
    let opts = parse_solve_opts(solve_opts_json)?;
    let world = World::from_save(&save, &defs);
    Ok(solve(&world, &defs, opts))
}

fn parse_solve_opts(json: &str) -> Result<SolveOpts, ApiError> {
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
///
/// Used by the browser defs builder: JS concatenates selected files and describes
/// slices in JSON. Paths must include `common/...` (or gfx) segments the loader expects.
///
/// # Errors
///
/// [`ApiError::Json`] or [`ApiError::DefsManifest`] when a slice is out of range.
pub fn manifest_files(
    manifest_json: &str,
    contents: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    let manifest: Vec<DefsFileEntry> = serde_json::from_str(manifest_json)?;
    manifest
        .into_iter()
        .map(|entry| {
            let end = entry.offset.checked_add(entry.length).ok_or_else(|| {
                ApiError::DefsManifest(format!("offset overflow for {}", entry.path))
            })?;
            let bytes = contents.get(entry.offset..end).ok_or_else(|| {
                ApiError::DefsManifest(format!(
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

/// Build a postcard definitions blob from a `{path, offset, length}` manifest.
///
/// # Errors
///
/// Manifest / defs load / encode failures.
pub fn build_defs_blob_bytes(manifest_json: &str, contents: &[u8]) -> Result<Vec<u8>, ApiError> {
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

/// Report what a definitions blob contains.
pub fn defs_summary_json(defs_blob: &[u8]) -> Result<String, ApiError> {
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

/// Goods and extra icons as PNG data URLs.
pub fn defs_icons_json(defs_blob: &[u8]) -> Result<String, ApiError> {
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

fn load_save(save_bytes: &[u8], tokens_bytes: Option<&[u8]>) -> Result<Save, ApiError> {
    match tokens_bytes {
        None | Some([]) => Ok(load_slice(save_bytes, empty_tokens())?),
        Some(tokens) => {
            let resolver = load_tokens_slice(tokens)?;
            Ok(load_slice(save_bytes, resolver)?)
        }
    }
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
    use vic3_load::{empty_tokens, load_slice, LoadError};
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
            matches!(err, ApiError::Load(LoadError::MissingTokens)),
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
        assert!(matches!(err, ApiError::Load(LoadError::MissingTokens)));
    }

    #[test]
    fn loaded_analysis_keeps_baseline_world_and_prices() {
        clear_analysis();
        assert!(matches!(
            loaded_prices_json(),
            Err(ApiError::NoLoadedAnalysis)
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
    fn loaded_optimize_pms_after_load_analysis_returns_axis() {
        clear_analysis();
        load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let json = loaded_optimize_pms_json(r#"{"axis":"income"}"#).expect("optimize");
        let v: Value = serde_json::from_str(&json).expect("OptimizeResult");
        assert_eq!(v["axis"], "income", "{json}");
        assert!(v["changes"].is_array(), "{json}");
        assert!(v["delta"]["income"].is_number(), "{json}");
        assert!(v["world_delta"].is_object(), "{json}");
        clear_analysis();
        assert!(matches!(
            loaded_optimize_pms_json(r#"{"axis":"income"}"#),
            Err(ApiError::NoLoadedAnalysis)
        ));
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
            Err(ApiError::NoLoadedAnalysis)
        ));
    }

    #[test]
    fn loaded_constructions_json_after_load_has_private_and_government_arrays() {
        clear_analysis();
        load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let json = loaded_constructions_json().expect("constructions snapshot");
        let v: Value = serde_json::from_str(&json).expect("constructions json");
        assert!(v["private"].is_array(), "{v}");
        assert!(v["government"].is_array(), "{v}");
        clear_analysis();
        assert!(matches!(
            loaded_constructions_json(),
            Err(ApiError::NoLoadedAnalysis)
        ));
    }

    #[test]
    fn export_save_patches_fixture_pms_without_mutating_origin() {
        let original = load_fixture();
        let before = original.clone();
        let patched = export_save_bytes(
            &original,
            r#"{"production_methods":[{"building_id":1,"methods":["pm_soil_enriching_farming","pm_no_automation"]}]}"#,
        )
        .expect("export");
        assert_eq!(original, before);
        assert_ne!(patched, original);
        let save = load_slice(&patched, empty_tokens()).expect("load patched");
        let farm = save
            .building_manager
            .database
            .get(&1)
            .and_then(|slot| slot.as_ref())
            .expect("building 1");
        assert_eq!(
            farm.active_production_methods(),
            ["pm_soil_enriching_farming", "pm_no_automation"]
        );
    }

    #[test]
    fn export_save_rejects_binary_sav_header() {
        let mut bytes = b"SAV0101deadbeef00000000\n".to_vec();
        bytes.extend_from_slice(&[0u8; 32]);
        let err = export_save_bytes(
            &bytes,
            r#"{"production_methods":[{"building_id":1,"methods":["pm_a"]}]}"#,
        )
        .expect_err("binary");
        let msg = err.to_string();
        assert!(
            msg.to_ascii_lowercase().contains("ironman")
                && msg.to_ascii_lowercase().contains("binary"),
            "{msg}"
        );
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
    fn loaded_production_methods_after_load_analysis() {
        clear_analysis();
        load_analysis_json(&load_fixture(), None, &defs_blob(), "{}").expect("load analysis");
        let json = loaded_production_methods_json().expect("loaded production methods");
        let methods: Vec<Value> = serde_json::from_str(&json).expect("PM array");
        assert!(
            methods.iter().any(|pm| pm["id"] == "pm_simple_forestry"),
            "{json}"
        );
        let forestry = methods
            .iter()
            .find(|pm| pm["id"] == "pm_simple_forestry")
            .expect("forestry");
        assert!(forestry["outputs"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()));
        clear_analysis();
        assert!(matches!(
            loaded_production_methods_json(),
            Err(ApiError::NoLoadedAnalysis)
        ));
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
                .is_some_and(|limitations| {
                    !limitations.is_empty()
                        && limitations.iter().any(|line| {
                            line.as_str() == Some(vic3_world::ARMY_POWER_PROJECTION_UNKNOWN)
                        })
                }),
            "army PP unknown limitation: {value}"
        );
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

    #[test]
    fn prices_from_paths_matches_bytes_api() {
        let save = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-load/tests/fixtures/plaintext.txt");
        let blob_path = std::env::temp_dir().join("vic3-api-test-defs.blob");
        let blob = defs_blob();
        std::fs::write(&blob_path, &blob).expect("write temp defs blob");
        let from_paths = prices_from_paths(&save, None, &blob_path, "{}").expect("paths");
        let from_bytes = prices_json(&load_fixture(), None, &blob, "{}").expect("bytes");
        assert_eq!(from_paths, from_bytes);
        let _ = std::fs::remove_file(&blob_path);
    }

    #[test]
    fn ensure_defs_blob_prefers_explicit_then_cache() {
        let tmp = std::env::temp_dir().join(format!("vic3-api-defs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let explicit = tmp.join("explicit.postcard");
        std::fs::write(&explicit, defs_blob()).unwrap();

        let resolved = ensure_defs_blob(Some(&explicit), None, &tmp).expect("explicit defs_blob");
        assert_eq!(resolved, explicit);

        let cache = tmp.join(DEFS_CACHE_NAME);
        std::fs::write(&cache, defs_blob()).unwrap();
        let from_cache = ensure_defs_blob(None, None, &tmp).expect("cached defs");
        assert_eq!(from_cache, cache);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_defs_blob_builds_from_game_fixture() {
        let tmp = std::env::temp_dir().join(format!("vic3-api-game-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let game = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures");
        let path = ensure_defs_blob(None, Some(&game), &tmp).expect("build cache");
        assert_eq!(path, tmp.join(DEFS_CACHE_NAME));
        assert!(path.is_file());
        // Second call must reuse the cache without requiring game_dir.
        let again = ensure_defs_blob(None, None, &tmp).expect("reuse cache");
        assert_eq!(again, path);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
