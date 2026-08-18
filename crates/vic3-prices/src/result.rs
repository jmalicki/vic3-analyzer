//! Option and result types matching `docs/json-schema.md` (`PricesResult`).

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Deref, Index};
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use vic3_defs::{GameDefs, GoodIdx, NeedIdx};

use crate::world::Intern;

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
    pub region_name: Option<String>,
    pub country_id: Option<u32>,
    pub market_id: Option<u32>,
    pub arable_land: Option<f64>,
    pub infrastructure: Option<f64>,
    pub infrastructure_usage: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatePop {
    pub state_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub profession_id: Option<String>,
    pub profession_name: Option<String>,
    pub demand_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workforce: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependents: Option<f64>,
    pub wealth: Option<i32>,
    pub culture_id: Option<String>,
    pub culture_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub literate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workplace_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub qualifications: Vec<ProfessionCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<PopNeedBasket>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfessionCount {
    pub profession_id: String,
    pub profession_name: Option<String>,
    pub count: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PopNeedBasket {
    pub need_id: String,
    pub need_name: Option<String>,
    pub package_value: f64,
    pub goods: Vec<GoodFlow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateNeed {
    pub state_id: u32,
    pub need_id: String,
    pub need_name: Option<String>,
    pub package_value: f64,
    pub goods: Vec<GoodFlow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateQualification {
    pub state_id: u32,
    pub profession_id: String,
    pub profession_name: Option<String>,
    pub qualified: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub employable: Option<f64>,
    pub employed: f64,
    pub jobs: f64,
    pub shortage: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_change: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BuildingTypeInfo {
    pub id: String,
    pub name: Option<String>,
    pub group_id: Option<String>,
    pub city_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BuildingGroupInfo {
    pub id: String,
    pub name: Option<String>,
    pub category: Option<String>,
    pub land_usage: Option<String>,
    pub always_possible: bool,
    pub default_building: Option<String>,
    pub parent_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateGood {
    pub state_id: u32,
    pub good_id: String,
    pub buy: f64,
    pub sell: f64,
    /// Final local price after blending market and pure state prices.
    pub price: f64,
    /// Solved whole-save market price (Milestone 1; per-market solves follow).
    pub market_price: f64,
    /// Pure state price from this state's attributed buy and sell orders.
    pub state_price: f64,
    /// Infrastructure-only state market access in `[0, 1]`.
    pub market_access: f64,
    /// Base MAPI 0.75 multiplied by [`Self::market_access`].
    pub effective_mapi: f64,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub employees: Vec<ProfessionCount>,
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
    /// Save pops dropped for missing `workforce`/`dependents` (or legacy
    /// population fields) or `wealth`.
    pub skipped_pops: usize,
    /// Buildings whose goods flows entered the solve.
    pub buildings: usize,
    /// Save buildings dropped for a missing type id.
    pub skipped_buildings: usize,
    /// Buildings with neither saved IO nor a production method present in the
    /// definitions.
    pub buildings_without_method: usize,
    /// Buildings with no non-zero saved IO and no usable PM fallback orders.
    pub buildings_without_orders: usize,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EmitTables {
    names: Intern,
    labels: BTreeMap<String, String>,
    goods_order: Vec<String>,
    needs_order: Vec<String>,
}

impl EmitTables {
    pub(crate) fn from_world(world: &crate::World, defs: &GameDefs) -> Self {
        Self {
            names: world.names.clone(),
            labels: defs.labels.clone(),
            goods_order: defs.goods_order.clone(),
            needs_order: defs.needs_order.clone(),
        }
    }

    pub(crate) fn name(&self, id: Option<u16>) -> Option<&str> {
        id.and_then(|id| self.names.get(id))
    }

    pub(crate) fn label(&self, id: &str) -> Option<&str> {
        self.labels.get(id).map(String::as_str)
    }

    pub(crate) fn good(&self, idx: GoodIdx) -> Option<&str> {
        self.goods_order.get(idx.as_usize()).map(String::as_str)
    }

    pub(crate) fn need(&self, idx: NeedIdx) -> Option<&str> {
        self.needs_order.get(idx.as_usize()).map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompactNeed {
    pub need_idx: NeedIdx,
    pub package_value: f64,
    pub goods: Vec<(GoodIdx, f64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompactStatePop {
    pub state_id: u32,
    pub id: Option<u32>,
    pub profession: Option<u16>,
    pub culture: Option<u16>,
    pub demand_size: Option<f64>,
    pub workforce: Option<f64>,
    pub dependents: Option<f64>,
    pub wealth: Option<i32>,
    pub literate: Option<f64>,
    pub workplace_id: Option<u32>,
    pub qualifications: Vec<(u16, f64)>,
    pub needs: Vec<CompactNeed>,
}

/// JSON `state_pops` array.
///
/// Solver rows stay interned (`u16` / [`GoodIdx`]) until serialize or first
/// field access. CLI default output never walks this list; wasm / `--json`
/// does. Skipping construction would make the CLI look fast while the webapp
/// still paid the cost.
#[derive(Clone, Debug)]
pub struct StatePopList {
    inner: StatePopStorage,
}

#[derive(Debug)]
enum StatePopStorage {
    Compact {
        tables: EmitTables,
        rows: Vec<CompactStatePop>,
        cache: OnceLock<Vec<StatePop>>,
    },
    Materialized(Vec<StatePop>),
}

impl Clone for StatePopStorage {
    fn clone(&self) -> Self {
        match self {
            Self::Materialized(rows) => Self::Materialized(rows.clone()),
            Self::Compact {
                tables,
                rows,
                cache,
            } => {
                let next = OnceLock::new();
                if let Some(rows) = cache.get() {
                    let _ = next.set(rows.clone());
                }
                Self::Compact {
                    tables: tables.clone(),
                    rows: rows.clone(),
                    cache: next,
                }
            }
        }
    }
}

impl Default for StatePopList {
    fn default() -> Self {
        Self {
            inner: StatePopStorage::Materialized(Vec::new()),
        }
    }
}

impl PartialEq for StatePopList {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl StatePopList {
    pub(crate) fn compact(tables: EmitTables, rows: Vec<CompactStatePop>) -> Self {
        Self {
            inner: StatePopStorage::Compact {
                tables,
                rows,
                cache: OnceLock::new(),
            },
        }
    }

    fn materialize_compact(tables: &EmitTables, rows: &[CompactStatePop]) -> Vec<StatePop> {
        rows.iter()
            .map(|row| materialize_state_pop(tables, row))
            .collect()
    }
}

impl Deref for StatePopList {
    type Target = [StatePop];

    fn deref(&self) -> &[StatePop] {
        match &self.inner {
            StatePopStorage::Materialized(rows) => rows,
            StatePopStorage::Compact {
                tables,
                rows,
                cache,
            } => cache.get_or_init(|| Self::materialize_compact(tables, rows)),
        }
    }
}

impl Index<usize> for StatePopList {
    type Output = StatePop;

    fn index(&self, index: usize) -> &Self::Output {
        &self.deref()[index]
    }
}

impl From<Vec<StatePop>> for StatePopList {
    fn from(rows: Vec<StatePop>) -> Self {
        Self {
            inner: StatePopStorage::Materialized(rows),
        }
    }
}

impl Serialize for StatePopList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.inner {
            StatePopStorage::Materialized(rows) => rows.serialize(serializer),
            StatePopStorage::Compact {
                tables,
                rows,
                cache,
            } => {
                if let Some(rows) = cache.get() {
                    return rows.serialize(serializer);
                }
                let mut seq = serializer.serialize_seq(Some(rows.len()))?;
                for row in rows {
                    seq.serialize_element(&StatePopSer::from_row(tables, row))?;
                }
                seq.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for StatePopList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Vec::<StatePop>::deserialize(deserializer)?.into())
    }
}

#[derive(Serialize)]
struct StatePopSer<'a> {
    state_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    profession_id: Option<&'a str>,
    profession_name: Option<&'a str>,
    demand_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workforce: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependents: Option<f64>,
    wealth: Option<i32>,
    culture_id: Option<&'a str>,
    culture_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    literate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workplace_id: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    qualifications: Vec<ProfessionCountSer<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    needs: Vec<NeedSer<'a>>,
}

#[derive(Serialize)]
struct ProfessionCountSer<'a> {
    profession_id: &'a str,
    profession_name: Option<&'a str>,
    count: f64,
}

#[derive(Serialize)]
struct NeedSer<'a> {
    need_id: &'a str,
    need_name: Option<&'a str>,
    package_value: f64,
    goods: Vec<GoodFlowSer<'a>>,
}

#[derive(Serialize)]
struct GoodFlowSer<'a> {
    good_id: &'a str,
    quantity: f64,
    value: f64,
}

impl<'a> StatePopSer<'a> {
    fn from_row(tables: &'a EmitTables, row: &'a CompactStatePop) -> Self {
        Self {
            state_id: row.state_id,
            id: row.id,
            profession_id: tables.name(row.profession),
            profession_name: tables.name(row.profession).and_then(|id| tables.label(id)),
            demand_size: row.demand_size,
            workforce: row.workforce,
            dependents: row.dependents,
            wealth: row.wealth,
            culture_id: tables.name(row.culture),
            culture_name: tables.name(row.culture).and_then(|id| tables.label(id)),
            literate: row.literate,
            workplace_id: row.workplace_id,
            qualifications: row
                .qualifications
                .iter()
                .filter(|(_, count)| *count > 0.0)
                .filter_map(|&(id, count)| {
                    let profession_id = tables.names.get(id)?;
                    Some(ProfessionCountSer {
                        profession_id,
                        profession_name: tables.label(profession_id),
                        count,
                    })
                })
                .collect(),
            needs: row
                .needs
                .iter()
                .filter_map(|need| {
                    let need_id = tables.need(need.need_idx)?;
                    Some(NeedSer {
                        need_id,
                        need_name: tables.label(need_id),
                        package_value: need.package_value,
                        goods: need
                            .goods
                            .iter()
                            .filter_map(|&(idx, quantity, value)| {
                                Some(GoodFlowSer {
                                    good_id: tables.good(idx)?,
                                    quantity,
                                    value,
                                })
                            })
                            .collect(),
                    })
                })
                .collect(),
        }
    }
}

fn materialize_state_pop(tables: &EmitTables, row: &CompactStatePop) -> StatePop {
    let ser = StatePopSer::from_row(tables, row);
    StatePop {
        state_id: ser.state_id,
        id: ser.id,
        profession_id: ser.profession_id.map(str::to_string),
        profession_name: ser.profession_name.map(str::to_string),
        demand_size: ser.demand_size,
        workforce: ser.workforce,
        dependents: ser.dependents,
        wealth: ser.wealth,
        culture_id: ser.culture_id.map(str::to_string),
        culture_name: ser.culture_name.map(str::to_string),
        literate: ser.literate,
        workplace_id: ser.workplace_id,
        qualifications: ser
            .qualifications
            .into_iter()
            .map(|row| ProfessionCount {
                profession_id: row.profession_id.to_string(),
                profession_name: row.profession_name.map(str::to_string),
                count: row.count,
            })
            .collect(),
        needs: ser
            .needs
            .into_iter()
            .map(|need| PopNeedBasket {
                need_id: need.need_id.to_string(),
                need_name: need.need_name.map(str::to_string),
                package_value: need.package_value,
                goods: need
                    .goods
                    .into_iter()
                    .map(|flow| GoodFlow {
                        good_id: flow.good_id.to_string(),
                        quantity: flow.quantity,
                        value: flow.value,
                    })
                    .collect(),
            })
            .collect(),
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
    pub building_types: Vec<BuildingTypeInfo>,
    pub building_groups: Vec<BuildingGroupInfo>,
    #[schemars(with = "Vec<StatePop>")]
    pub state_pops: StatePopList,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_qualifications: Vec<StateQualification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_needs: Vec<StateNeed>,
    /// Where the orders behind these prices came from.
    pub inputs: MarketInputs,
    /// `‖r − r_formula(orders(r))‖₂`. Always present (I5).
    pub residual: f64,
    pub status: SolveStatus,
    pub limitations: Vec<String>,
}
