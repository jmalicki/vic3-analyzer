//! In-browser facade: thin `#[wasm_bindgen]` over [`vic3_api`].
//!
//! # Role
//!
//! No filesystem. The page (or worker) supplies save / token / defs **bytes**;
//! this crate only adapts [`vic3_api`] to `wasm_bindgen` and maps [`vic3_api::ApiError`]
//! to [`JsError`]. clap never links here. Option and result JSON match CLI / Tauri /
//! MCP hosts that call the same `*_json` functions.
//!
//! # Bindgen surface
//!
//! | Export | API | Notes |
//! | --- | --- | --- |
//! | [`parse_save`] | [`vic3_api::parse_save_json`] | summary only |
//! | [`load_analysis`] | [`vic3_api::load_analysis_json`] | installs worker session |
//! | [`clear_analysis`] | [`vic3_api::clear_analysis`] | |
//! | [`loaded_prices`] … [`loaded_plan`] | matching `loaded_*_json` | need a session |
//! | [`prices`] / [`what_if`] / [`gaps`] / [`plan`] / [`alerts`] | one-shot `*_json` | no session |
//! | [`export_save`] | [`vic3_api::export_save_bytes`] | plaintext only |
//! | [`build_defs_blob`] / [`DefsBlobBuilder`] | manifest → postcard | streaming for large installs |
//! | [`defs_summary`] / [`defs_icons`] | blob introspection | |
//! | [`what_if_schema`] / [`prices_schema`] | schemars for React forms | |
//!
//! Exports return JSON **text** (not `JsValue`) so native tests round-trip the
//! same payload the CLI prints. Original `.v3` bytes stay in JS/IndexedDB;
//! the session holds IR + world + baseline prices only.
//!
//! # Build
//!
//! ```text
//! wasm-pack build crates/vic3-wasm --target web --out-dir web/public/wasm
//! ```
//!
//! (or `npm run build:wasm` from `web/`). Compiles for `wasm32-unknown-unknown`
//! without extra `getrandom` feature flags on the current dependency set.
//!
//! See [`docs/cli.md`](../../../docs/cli.md) and [`docs/architecture.md`](../../../docs/architecture.md).

mod schema;

use wasm_bindgen::prelude::*;

pub use vic3_api::ApiError as WasmError;

/// Crate version from Cargo.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Parse a `.v3` (and optional token map) into an IR summary JSON.
///
/// Empty / omitted `tokens_bytes` is correct for plaintext. Binary saves with
/// no tokens return a missing-tokens load error (`vic3_load::LoadError::MissingTokens`).
#[wasm_bindgen]
pub fn parse_save(save_bytes: &[u8], tokens_bytes: Option<Vec<u8>>) -> Result<String, JsError> {
    vic3_api::parse_save_json(save_bytes, tokens_bytes.as_deref()).map_err(to_js)
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
    vic3_api::load_analysis_json(
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
    vic3_api::clear_analysis();
}

/// Return the prices computed while loading the current analysis.
#[wasm_bindgen]
pub fn loaded_prices() -> Result<String, JsError> {
    vic3_api::loaded_prices_json().map_err(to_js)
}

/// Return a conservative military snapshot for the played country.
#[wasm_bindgen]
pub fn loaded_military() -> Result<String, JsError> {
    vic3_api::loaded_military_json().map_err(to_js)
}

/// Return private + government construction queues for the played country.
#[wasm_bindgen]
pub fn loaded_constructions() -> Result<String, JsError> {
    vic3_api::loaded_constructions_json().map_err(to_js)
}

/// Patch plaintext `.v3` bytes with building production methods and extra levels.
///
/// `original_bytes` are the origin save from JS (IndexedDB). They are not
/// mutated; the return is a new buffer. Ironman/binary saves error.
/// `delta_json` is a `vic3_load::SavePatch` JSON object.
#[wasm_bindgen]
pub fn export_save(original_bytes: &[u8], delta_json: &str) -> Result<Vec<u8>, JsError> {
    vic3_api::export_save_bytes(original_bytes, delta_json).map_err(to_js)
}

/// Apply a what-if delta to the current worker-owned world.
#[wasm_bindgen]
pub fn loaded_what_if(what_if_opts_json: &str) -> Result<String, JsError> {
    vic3_api::loaded_what_if_json(what_if_opts_json).map_err(to_js)
}

/// Apply a `vic3_prices::WorldDelta` to a clone of the loaded world and re-solve (preview).
///
/// Does not replace the loaded world or baseline prices, and does not write a save.
#[wasm_bindgen]
pub fn loaded_apply_delta(delta_json: &str) -> Result<String, JsError> {
    vic3_api::loaded_apply_delta_json(delta_json).map_err(to_js)
}

/// Suggest production-method changes for the loaded world (`OptimizeResult` JSON).
///
/// `axis_json` is `{"axis":"income"}`, `{"axis":"productivity"}`, or `{"axis":"sol"}`.
/// Does not replace the loaded world or write a save.
#[wasm_bindgen]
pub fn loaded_optimize_pms(axis_json: &str) -> Result<String, JsError> {
    vic3_api::loaded_optimize_pms_json(axis_json).map_err(to_js)
}

/// Evaluate goal gaps against the current worker-owned analysis.
#[wasm_bindgen]
pub fn loaded_gaps(goal: &str) -> Result<String, JsError> {
    vic3_api::loaded_gaps_json(goal).map_err(to_js)
}

/// Plan from the current worker-owned analysis.
#[wasm_bindgen]
pub fn loaded_plan(plan_opts_json: &str) -> Result<String, JsError> {
    vic3_api::loaded_plan_json(plan_opts_json).map_err(to_js)
}

/// Diagnose shortages from the current worker-owned analysis (`AlertsResult` JSON).
#[wasm_bindgen]
pub fn loaded_alerts() -> Result<String, JsError> {
    vic3_api::loaded_alerts_json().map_err(to_js)
}

/// Production-method recipes from the loaded definitions (`[{id, inputs, outputs}]`).
#[wasm_bindgen]
pub fn loaded_production_methods() -> Result<String, JsError> {
    vic3_api::loaded_production_methods_json().map_err(to_js)
}

/// Build a postcard definitions blob from browser-selected Victoria 3 files.
///
/// `manifest_json` is an array of `{path, offset, length}` entries into the
/// concatenated `contents` payload. Paths must include `common/...`.
#[wasm_bindgen]
pub fn build_defs_blob(manifest_json: &str, contents: &[u8]) -> Result<Vec<u8>, JsError> {
    vic3_api::build_defs_blob_bytes(manifest_json, contents).map_err(to_js)
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
        let files = vic3_api::manifest_files(manifest_json, contents).map_err(to_js)?;
        self.inner
            .add_files(files)
            .map_err(|error| to_js(WasmError::from(error)))?;
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
    vic3_api::defs_summary_json(defs_blob).map_err(to_js)
}

/// Goods and extra icons as PNG data URLs, ready for an `img` tag.
///
/// Shape: `{ grain: url, goods: { grain: url }, extra: { "building:foo": url } }`.
/// Goods are nested and also flattened at the top level for older UI.
#[wasm_bindgen]
pub fn defs_icons(defs_blob: &[u8]) -> Result<String, JsError> {
    vic3_api::defs_icons_json(defs_blob).map_err(to_js)
}

/// Solve market prices. `defs_blob` is a postcard blob from `vic3_defs::encode_blob`.
/// `solve_opts_json` is a `vic3_prices::SolveOpts` object; empty / `{}` uses defaults.
#[wasm_bindgen]
pub fn prices(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
) -> Result<String, JsError> {
    vic3_api::prices_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
    )
    .map_err(to_js)
}

/// Apply a what-if building-level delta and re-solve.
///
/// `what_if_opts_json` is `vic3_prices::WhatIfOpts` (`building`, `extra_levels`).
#[wasm_bindgen]
pub fn what_if(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    what_if_opts_json: &str,
) -> Result<String, JsError> {
    vic3_api::what_if_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
        what_if_opts_json,
    )
    .map_err(to_js)
}

/// JSON Schema for `vic3_prices::WhatIfOpts` (UI form).
#[wasm_bindgen]
pub fn what_if_schema() -> String {
    schema::what_if_schema_json()
}

/// JSON Schema for `vic3_prices::PricesResult` (includes `residual` and `limitations`).
#[wasm_bindgen]
pub fn prices_schema() -> String {
    schema::prices_schema_json()
}

/// Find a shortest goal-relevant plan and return `vic3_planning::PlanResult` JSON.
#[wasm_bindgen]
pub fn plan(
    save_bytes: &[u8],
    tokens_bytes: Option<Vec<u8>>,
    defs_blob: &[u8],
    solve_opts_json: &str,
    plan_opts_json: &str,
) -> Result<String, JsError> {
    vic3_api::plan_json(
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
    vic3_api::gaps_json(
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
    vic3_api::alerts_json(
        save_bytes,
        tokens_bytes.as_deref(),
        defs_blob,
        solve_opts_json,
    )
    .map_err(to_js)
}

// Thin re-exports for native callers that historically linked `vic3-wasm`.
pub use vic3_api::{
    alerts_json, build_defs_blob_bytes, defs_icons_json, defs_summary_json, export_save_bytes,
    gaps_json, load_analysis_json, loaded_alerts_json, loaded_apply_delta_json,
    loaded_constructions_json, loaded_gaps_json, loaded_military_json, loaded_optimize_pms_json,
    loaded_plan_json, loaded_prices_json, loaded_production_methods_json, loaded_what_if_json,
    parse_save_json, plan_json, prices_json, what_if_json,
};

fn to_js(err: WasmError) -> JsError {
    JsError::new(&err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

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
            classify_defs_path("game/gfx/interface/icons/pops_icons/academics.dds", false),
            "read"
        );
        assert_eq!(
            classify_defs_path(
                "game/gfx/interface/icons/ships/ship_types/silhouette_frigate.dds",
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
