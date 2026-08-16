//! Option and result types matching `docs/json-schema.md` (`PricesResult`).

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Solver iteration / residual tolerances.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolveOpts {
    /// Residual threshold for [`SolveStatus::Converged`] (I5). Default `1e-6`.
    #[serde(default = "default_residual_eps")]
    pub residual_eps: f64,
    /// Combined successive-substitution + Basin iteration cap. Default `100`.
    #[serde(default = "default_max_iters")]
    #[schemars(range(min = 1))]
    pub max_iters: u32,
}

fn default_residual_eps() -> f64 {
    1e-6
}

fn default_max_iters() -> u32 {
    100
}

impl Default for SolveOpts {
    fn default() -> Self {
        Self {
            residual_eps: default_residual_eps(),
            max_iters: default_max_iters(),
        }
    }
}

/// Extra building levels applied before a re-solve. Employment stays frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WhatIfOpts {
    /// Building type id (matches [`crate::WorldBuilding::building`]).
    pub building: String,
    /// Non-negative extra levels added to matching buildings.
    #[schemars(range(min = 0))]
    pub extra_levels: u32,
}

/// Why [`crate::solve`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    /// [`PricesResult::residual`] is below [`SolveOpts::residual_eps`] (I5).
    Converged,
    /// Iteration budget exhausted with residual still at or above ε.
    MaxIters,
    /// Basin reported failure and successive substitution did not recover.
    Failed,
}

impl fmt::Display for SolveStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Converged => "converged",
            Self::MaxIters => "max_iters",
            Self::Failed => "failed",
        })
    }
}

/// One row of the goods table in [`PricesResult`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoodPrice {
    pub id: String,
    pub name: Option<String>,
    pub base: f64,
    pub price: f64,
    pub buy: f64,
    pub sell: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CountryInfo {
    pub id: u32,
    pub tag: String,
    pub name: Option<String>,
    /// Selected coat-of-arms id when a current flag could be determined.
    pub flag_coa: Option<String>,
    /// PNG data URL for the selected flag, when the defs blob rendered it.
    pub flag_data_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateInfo {
    pub id: u32,
    pub region_id: Option<String>,
    pub country_id: Option<u32>,
    pub market_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateGood {
    pub state_id: u32,
    pub good_id: String,
    pub buy: f64,
    pub sell: f64,
    /// Shared whole-save synthetic price. This is not a MAPI local price.
    pub price: f64,
    pub base: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoodFlow {
    pub good_id: String,
    pub quantity: f64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BuildingEconomics {
    pub id: u32,
    pub state_id: Option<u32>,
    pub type_id: String,
    pub level: f64,
    pub staffing: f64,
    pub production_method_ids: Vec<String>,
    pub inputs: Vec<GoodFlow>,
    pub outputs: Vec<GoodFlow>,
    pub revenue: f64,
    pub cost: f64,
    pub profit: f64,
    pub short_inputs: Vec<String>,
}

/// What the save and definitions actually contributed to the market.
///
/// Every price equals its base price when nothing places an order, and that
/// solve reports `converged` with a zero residual. These counts tell the two
/// cases apart: a genuinely balanced market versus a market with no orders in
/// it because the save or the definitions did not supply any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct MarketInputs {
    /// Pops whose consumption entered the solve.
    pub pops: usize,
    /// Save pops dropped for missing `size_wa`/`size_dn` (or legacy `size`) or
    /// `wealth`.
    pub skipped_pops: usize,
    /// Buildings whose goods flows entered the solve.
    pub buildings: usize,
    /// Save buildings dropped for a missing type id.
    pub skipped_buildings: usize,
    /// Buildings whose production methods are all absent from the definitions,
    /// so they neither consume nor produce.
    pub buildings_without_method: usize,
    /// Goods carrying a non-zero buy or sell order. Zero means every price
    /// below is just its base price.
    pub goods_with_orders: usize,
}

impl MarketInputs {
    /// Whether the solve had any order to price at all.
    pub fn is_empty_market(&self) -> bool {
        self.goods_with_orders == 0
    }
}

/// Price-equilibrium output. `limitations` is always the crate [`crate::LIMITATIONS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PricesResult {
    /// Prices currently solve one synthetic economy for the whole save.
    pub scope: String,
    pub goods: Vec<GoodPrice>,
    pub countries: Vec<CountryInfo>,
    pub states: Vec<StateInfo>,
    pub state_goods: Vec<StateGood>,
    pub buildings: Vec<BuildingEconomics>,
    /// Where the orders behind these prices came from.
    pub inputs: MarketInputs,
    /// `‖r − r_formula(orders(r))‖₂`. Always present (I5).
    pub residual: f64,
    pub status: SolveStatus,
    pub limitations: Vec<String>,
}
