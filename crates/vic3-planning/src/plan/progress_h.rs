//! Progress-aware ranking heuristic library (not wired into search yet).
//!
//! # Role in search
//!
//! Wired A\* still orders Ready keys with the **admissible timing**
//! heuristic on [`super::Vic3Node`] (calendar-day lower bound from the timing
//! DAG). That bound is correct but blind to *how much* of a goal meter (e.g.
//! GDP) remains.
//!
//! This module estimates **residual calendar days to close the goal**, using
//! gaps, construction throughput, and cheap GDP / Construction Sector
//! guesstimates. Those scores are meant only to **rank** the open set /
//! candidate bag so search prefers actions that make progress. They are **not**
//! a proven admissible lower bound and must not be used as an incumbent upper
//! bound (greedy upper bounds are a later PR).
//!
//! Nothing here is called from `plan()` / search yet -- wiring lands later.
//! Design write-up: `docs/planning-progress-heuristic.md`.
//!
//! # Timeline naming
//!
//! - **Current state** (`state_t` in the doc): the node being expanded.
//! - **Successor** (`state_{t'}`): after applying an action (may share the same
//!   calendar day when the edge costs 0 days).
//! - Rust suffixes: `*_curr` for values computed on the current state and reused
//!   for every candidate in one expand; avoid “parent” for bag GDP math.
//!
//! # Design-doc names ↔ Rust
//!
//! | Design doc | Rust |
//! | --- | --- |
//! | Current-state bag context | [`CheapBagCurr`] |
//! | Follow-on residual, meter gap, aggregate rate | [`BagResidualCurr`] (`follow_on_days`, `meter_gap`, `aggregate_rate`) |
//! | Cheap GDP Δ guesstimate for an ordinary build | [`cheap_gdp_delta_guesstimate`] |
//! | Emit residual on a **completed** planning world | [`emit_bag_score`] |
//! | Construction throughput (points/day) | `construction_points_per_day` on [`PlanningState`] |

#![allow(dead_code)] // unwired library; search / greedy call sites land in later PRs

use crate::construction::{
    allocation_cap_points_per_day, construction_add_for_cs_building, construction_eta_days,
    construction_work_points_for_enqueue, government_construction_share_from_laws,
    max_parallel_construction_jobs, ConstructionEtaMode, BUILDING_CONSTRUCTION_SECTOR,
};
use crate::goals::{Goal, Rel, SimpleSubgoal};
use crate::sim::{Action, EconomyContext, SimConfig};
use crate::world::PlanningState;

/// Failure while measuring a subgoal gap on planning state.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProgressGapError {
    #[error("planning state missing {meter} needed to evaluate subgoal")]
    MissingMeter { meter: &'static str },
}

/// How far a simple subgoal still is from satisfaction on `state`.
///
/// Returns the non-negative shortfall in the subgoal’s native units (GDP,
/// price, power, …). Binary subgoals (`HasTech`, `HasLaw`, …) use `0.0` when
/// already true and `1.0` when false.
///
/// Returns [`ProgressGapError::MissingMeter`] when the planning state does not
/// carry the meter needed to evaluate the subgoal (e.g. no recorded good
/// price). In practice that means the planner never loaded that field from the
/// save/world -- ranking cannot invent a gap, so callers must bubble the error
/// rather than treating it as “already satisfied.”
pub fn simple_subgoal_gap(
    subgoal: &SimpleSubgoal,
    state: &PlanningState,
) -> Result<f64, ProgressGapError> {
    match subgoal {
        SimpleSubgoal::Gdp { rel, value } => Ok(raise_gap(state.gdp, *rel, *value)),
        SimpleSubgoal::GoodPrice { good, rel, value } => {
            let price = state.price(good).ok_or(ProgressGapError::MissingMeter {
                meter: "good price",
            })?;
            Ok(band_or_raise_gap(price, *rel, *value))
        }
        SimpleSubgoal::ArmyPower { rel, value } => {
            let power = state
                .army_power_projection
                .ok_or(ProgressGapError::MissingMeter {
                    meter: "army power projection",
                })?;
            Ok(raise_gap(power, *rel, *value))
        }
        SimpleSubgoal::NavyPower { rel, value } => {
            let power = state
                .navy_power_projection
                .ok_or(ProgressGapError::MissingMeter {
                    meter: "navy power projection",
                })?;
            Ok(raise_gap(power, *rel, *value))
        }
        SimpleSubgoal::WeeklyBalance { rel, value } => {
            let bal = state.weekly_balance.ok_or(ProgressGapError::MissingMeter {
                meter: "weekly balance",
            })?;
            Ok(raise_gap(bal, *rel, *value))
        }
        SimpleSubgoal::PopulationWeightedWealth { rel, value } => {
            let w = state
                .population_weighted_wealth
                .ok_or(ProgressGapError::MissingMeter {
                    meter: "population-weighted wealth",
                })?;
            Ok(raise_gap(w, *rel, *value))
        }
        SimpleSubgoal::DebtPrincipal { rel, value } => {
            let d = state.debt_principal.ok_or(ProgressGapError::MissingMeter {
                meter: "debt principal",
            })?;
            Ok(raise_gap(d, *rel, *value))
        }
        SimpleSubgoal::CreditHeadroom { rel, value } => {
            let headroom = state
                .credit_headroom
                .ok_or(ProgressGapError::MissingMeter {
                    meter: "credit headroom",
                })?;
            Ok(raise_gap(headroom, *rel, *value))
        }
        SimpleSubgoal::HasTech(tech) => Ok(if state.has_tech(tech) { 0.0 } else { 1.0 }),
        SimpleSubgoal::HasLaw(law) => Ok(if state.has_law(law) { 0.0 } else { 1.0 }),
        SimpleSubgoal::InterestIn { kind, id } => {
            let held = match kind {
                crate::goals::InterestKind::State => state.has_interest_state(id),
                crate::goals::InterestKind::Region => state.has_interest_region(id),
            };
            Ok(if held { 0.0 } else { 1.0 })
        }
        SimpleSubgoal::Solvent => Ok(if state.solvent { 0.0 } else { 1.0 }),
    }
}

/// Like [`simple_subgoal_gap`], but GDP leaves read `gdp_for_rates` instead of
/// `state.gdp`.
///
/// Only GDP is special-cased: emit/search often keep the child’s **enqueue-only**
/// `state.gdp` while ranking as if a build had already completed. Anticipated GDP
/// must flow into residual math without mutating that child. Other subgoal kinds
/// still read meters from `state` unchanged.
fn simple_subgoal_gap_with_gdp(
    subgoal: &SimpleSubgoal,
    state: &PlanningState,
    gdp_for_rates: f64,
) -> Result<f64, ProgressGapError> {
    match subgoal {
        SimpleSubgoal::Gdp { rel, value } => Ok(raise_gap(gdp_for_rates, *rel, *value)),
        _ => simple_subgoal_gap(subgoal, state),
    }
}

/// Non-negative shortfall of `current` vs `target` under `rel`.
///
/// Uses [`Rel::holds`] as the sole satisfaction check (including `Rel::Eq`
/// tolerance). When the relation does not hold, returns a **positive**
/// directional shortfall -- including a tiny epsilon for strict `Gt`/`Lt` when
/// `current` sits exactly on the boundary (where a naive `max(0, target -
/// current)` would be zero even though `holds` is false).
fn raise_gap(current: f64, rel: Rel, target: f64) -> f64 {
    if rel.holds(current, target) {
        return 0.0;
    }
    let eps = (1e-9_f64).max(1e-9 * (1.0 + target.abs()));
    match rel {
        Rel::Ge => (target - current).max(0.0),
        Rel::Gt => {
            let d = target - current;
            if d > 0.0 {
                d
            } else {
                eps
            }
        }
        Rel::Le => (current - target).max(0.0),
        Rel::Lt => {
            let d = current - target;
            if d > 0.0 {
                d
            } else {
                eps
            }
        }
        Rel::Eq => (current - target).abs(),
    }
}

/// Price-band shortfall; same raise/lower/eq math as [`raise_gap`] today.
fn band_or_raise_gap(current: f64, rel: Rel, target: f64) -> f64 {
    raise_gap(current, rel, target)
}

/// Progress-aware remaining-days estimate for ranking.
///
/// **Contract:** this is a ranking bias, **not** an admissible A\* heuristic.
/// Implementations may overestimate remaining days; they are **not** required
/// to underestimate (admissible lower bounds live on the timing DAG `h`).
///
/// Walks the goal tree: `And` takes the max child residual, `Or` the min,
/// `Not` contributes 0. GDP gaps use `state.gdp`.
///
/// Prefer [`rank_heuristic_with_gdp_for_rates`] when the caller already knows an
/// anticipated GDP (emit after speculative complete).
pub fn rank_heuristic(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
) -> Result<u32, ProgressGapError> {
    rank_heuristic_with_gdp_for_rates(goal, state, config, economy, state.gdp)
}

/// Like [`rank_heuristic`], but GDP gaps use `gdp_for_rates` instead of `state.gdp`.
///
/// `gdp_for_rates` is the GDP plugged into residual-days math. Emit can pass
/// `state.gdp + anticipated_delta` before `BuildingCompleted` refreshes the
/// child’s stored GDP.
///
/// Goal-tree fold:
/// - `And`: worst (max) child residual -- every conjunct must clear.
/// - `Or`: best (min) child residual -- any disjunct suffices.
/// - `Not`: 0 (no progress residual modeled for negated goals today).
/// - `Simple`: [`rank_simple_subgoal`].
pub fn rank_heuristic_with_gdp_for_rates(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
    gdp_for_rates: f64,
) -> Result<u32, ProgressGapError> {
    match goal {
        Goal::And(children) => Ok(children
            .iter()
            .map(|child| {
                rank_heuristic_with_gdp_for_rates(child, state, config, economy, gdp_for_rates)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .unwrap_or(0)),
        Goal::Or(children) => Ok(children
            .iter()
            .map(|child| {
                rank_heuristic_with_gdp_for_rates(child, state, config, economy, gdp_for_rates)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min()
            .unwrap_or(0)),
        Goal::Not(_) => Ok(0),
        Goal::Simple(subgoal) => {
            rank_simple_subgoal(subgoal, state, config, economy, gdp_for_rates)
        }
    }
}

/// Residual days for one simple subgoal leaf (meter gaps to days; tech/law/
/// interest use track ETAs).
fn rank_simple_subgoal(
    subgoal: &SimpleSubgoal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
    gdp_for_rates: f64,
) -> Result<u32, ProgressGapError> {
    // GDP satisfaction must use gdp_for_rates (same meter as the gap), not
    // state.gdp / subgoal.eval, which still see enqueue-only GDP.
    let satisfied = match subgoal {
        SimpleSubgoal::Gdp { rel, value } => rel.holds(gdp_for_rates, *value),
        _ => subgoal.eval(state),
    };
    if satisfied {
        return Ok(0);
    }
    match subgoal {
        SimpleSubgoal::Gdp { .. }
        | SimpleSubgoal::GoodPrice { .. }
        | SimpleSubgoal::ArmyPower { .. }
        | SimpleSubgoal::NavyPower { .. }
        | SimpleSubgoal::Solvent
        | SimpleSubgoal::WeeklyBalance { .. }
        | SimpleSubgoal::PopulationWeightedWealth { .. }
        | SimpleSubgoal::DebtPrincipal { .. }
        | SimpleSubgoal::CreditHeadroom { .. } => {
            let gap = simple_subgoal_gap_with_gdp(subgoal, state, gdp_for_rates)?;
            if gap <= 0.0 {
                return Ok(0);
            }
            Ok(residual_days_from_gap(
                gap,
                state,
                config,
                economy,
                subgoal,
                gdp_for_rates,
            ))
        }
        SimpleSubgoal::HasTech(tech) => Ok(research_eta_for_rank(tech, state, config, economy)),
        SimpleSubgoal::HasLaw(_) => Ok(u32::from(config.law_days.max(1))),
        SimpleSubgoal::InterestIn { .. } => Ok(u32::from(config.interest_days.max(1))),
    }
}

/// Serial research ETA for ranking: sum of missing tech costs at a constant
/// research rate (mirrors the timing-leaf idea; kept local so this module does
/// not depend on `vic3` crate visibility).
///
/// Uses [`u32::saturating_add`] when summing prereq ETAs (same no-wrap policy
/// as bag score day sums).
fn research_eta_for_rank(
    tech: &str,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
) -> u32 {
    if state.has_tech(tech) {
        return 0;
    }
    let fallback = u32::from(config.research_days.max(1));
    if economy.defs.technologies.is_empty() {
        return fallback;
    }
    let missing = crate::tech::missing_tech_closure(tech, state, &economy.defs);
    if missing.is_empty() {
        return 0;
    }
    let mut total = 0u32;
    for id in missing {
        total = total.saturating_add(single_tech_research_eta(&id, config, economy));
    }
    total.max(1)
}

/// Days to research one tech from defs cost, or `config.research_days` fallback.
fn single_tech_research_eta(tech: &str, config: SimConfig, economy: &EconomyContext) -> u32 {
    let fallback = u32::from(config.research_days.max(1));
    if let Some(cost) = crate::tech::tech_research_cost(tech, &economy.defs) {
        if let Some(days) = crate::tracks::days_for_work(cost, crate::tracks::CONSTANT_RATE) {
            return days.max(1);
        }
    }
    fallback
}

/// Convert a meter gap into an approximate residual calendar-day count.
///
/// Uses construction ETA * parallel feed slots as a stand-in aggregate
/// “progress per day,” then `ceil(gap / rate)`. Coarse by design -- ranking
/// only.
fn residual_days_from_gap(
    gap: f64,
    state: &PlanningState,
    config: SimConfig,
    _economy: &EconomyContext,
    subgoal: &SimpleSubgoal,
    gdp_for_rates: f64,
) -> u32 {
    let eta = construction_eta_days(state, config, ConstructionEtaMode::CapacityOrSlot).max(1);
    let cap = allocation_cap_points_per_day(state, config, None);
    let slots = max_parallel_construction_jobs(state.construction_points_per_day, cap).get() as f64;

    let ref_delta = match subgoal {
        SimpleSubgoal::Gdp { .. } => (gdp_for_rates.abs() * 0.005).max(gap * 0.05).max(1.0),
        SimpleSubgoal::ArmyPower { .. } | SimpleSubgoal::NavyPower { .. } => gap.max(1.0),
        SimpleSubgoal::GoodPrice { .. } => gap.max(0.5),
        _ => gap.max(1.0),
    };
    let aggregate_rate = (ref_delta / f64::from(eta)) * slots;
    if aggregate_rate <= 1e-12 {
        return eta;
    }
    let days = (gap / aggregate_rate).ceil();
    if !days.is_finite() || days <= 0.0 {
        return eta;
    }
    (days as u32).clamp(1, u32::MAX / 4)
}

/// Follow-on residual days on the **current** timeline state at today’s
/// construction capacity (design-doc follow-on residual).
///
/// This is a **named alias** for [`rank_heuristic_with_gdp_for_rates`]: same
/// arguments and result. Bag/CS code calls this so call sites read as “follow-on
/// days on the current node,” not as a second ranking algorithm. If follow-on
/// ever diverges from full rank (e.g. excluding track leaves), change this body
/// -- do not fork a silent second formula at call sites.
pub fn follow_on_days_curr(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
    gdp_for_rates: f64,
) -> Result<u32, ProgressGapError> {
    rank_heuristic_with_gdp_for_rates(goal, state, config, economy, gdp_for_rates)
}

/// Aggregate meter progress per day implied by the current residual
/// (`gap / follow_on_days` style).
///
/// Used when cheap ordinary-build scoring credits a GDP guesstimate and needs
/// to turn the remaining gap back into days:
/// `ceil(remaining_gap / aggregate_rate)`.
pub fn aggregate_rate_curr(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
    gdp_for_rates: f64,
) -> Result<f64, ProgressGapError> {
    let mut total_gap = 0.0_f64;
    for subgoal in goal.simple_subgoals() {
        match subgoal {
            SimpleSubgoal::Gdp { .. }
            | SimpleSubgoal::GoodPrice { .. }
            | SimpleSubgoal::ArmyPower { .. }
            | SimpleSubgoal::NavyPower { .. } => {
                let gap = simple_subgoal_gap_with_gdp(subgoal, state, gdp_for_rates)?;
                total_gap += gap.max(0.0);
            }
            _ => {}
        }
    }
    if total_gap <= 1e-12 {
        return Ok(1.0);
    }
    let follow = residual_days_from_gap(
        total_gap,
        state,
        config,
        economy,
        &SimpleSubgoal::Gdp {
            rel: Rel::Ge,
            value: gdp_for_rates + total_gap,
        },
        gdp_for_rates,
    )
    .max(1);
    Ok(total_gap / f64::from(follow))
}

/// Sum of open **GDP** leaf gaps on `goal`, using `gdp_for_rates`.
///
/// Used as the meter that [`cheap_bag_score`] may reduce with a GDP guesstimate.
/// Non-GDP leaves (tech, law, interest, fiscal, …) are intentionally omitted
/// here so a wood-camp guesstimate cannot erase unrelated residuals; those
/// remain in [`follow_on_days_curr`] / full rank. Missing GDP meters bubble
/// [`ProgressGapError`].
pub fn meter_gap_curr(
    goal: &Goal,
    state: &PlanningState,
    gdp_for_rates: f64,
) -> Result<f64, ProgressGapError> {
    let mut total = 0.0_f64;
    for subgoal in goal.simple_subgoals() {
        if matches!(subgoal, SimpleSubgoal::Gdp { .. }) {
            let gap = simple_subgoal_gap_with_gdp(subgoal, state, gdp_for_rates)?;
            total += gap.max(0.0);
        }
    }
    Ok(total)
}

/// True when every **open** simple leaf is a GDP goal (so cheap GDP credit is safe).
///
/// If tech/law/interest/fiscal (etc.) still have a positive gap, ordinary-build
/// cheap scoring must not subtract a GDP guesstimate from the bag residual.
fn allow_cheap_gdp_credit(
    goal: &Goal,
    state: &PlanningState,
    gdp_for_rates: f64,
) -> Result<bool, ProgressGapError> {
    for subgoal in goal.simple_subgoals() {
        let gap = simple_subgoal_gap_with_gdp(subgoal, state, gdp_for_rates)?;
        if gap > 0.0 && !matches!(subgoal, SimpleSubgoal::Gdp { .. }) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Precomputed residual inputs for one candidate-bag expand on the current state.
///
/// Built once via [`BagResidualCurr::compute`], then shared by every
/// [`cheap_bag_score`] call for candidates from that expand so follow-on days,
/// gap, and aggregate rate stay consistent across the bag.
#[derive(Debug, Clone, Copy)]
pub struct BagResidualCurr {
    /// Estimated residual days to clear the goal at **current** capacity
    /// ([`follow_on_days_curr`]).
    pub follow_on_days: u32,
    /// Open GDP meter gap only ([`meter_gap_curr`]) for cheap GDP credit.
    pub meter_gap: f64,
    /// Implied meter units per day ([`aggregate_rate_curr`]) for converting
    /// remaining GDP gap to days.
    pub aggregate_rate: f64,
    /// When false, ordinary-build cheap scoring must not subtract a GDP
    /// guesstimate (non-GDP leaves still open).
    pub allow_cheap_gdp_credit: bool,
}

impl BagResidualCurr {
    /// Snapshot follow-on days, GDP meter gap, aggregate rate, and credit flag.
    pub fn compute(
        goal: &Goal,
        state: &PlanningState,
        config: SimConfig,
        economy: &EconomyContext,
        gdp_for_rates: f64,
    ) -> Result<Self, ProgressGapError> {
        Ok(Self {
            follow_on_days: follow_on_days_curr(goal, state, config, economy, gdp_for_rates)?,
            meter_gap: meter_gap_curr(goal, state, gdp_for_rates)?,
            aggregate_rate: aggregate_rate_curr(goal, state, config, economy, gdp_for_rates)?,
            allow_cheap_gdp_credit: allow_cheap_gdp_credit(goal, state, gdp_for_rates)?,
        })
    }
}

/// Shared handles for scoring every cheap bag candidate in one expand.
///
/// Holds the current planning state, sim config, economy (defs / world for
/// construction and GDP guesstimates), and the precomputed [`BagResidualCurr`].
pub struct CheapBagCurr<'a> {
    /// Node being expanded (current timeline state).
    pub state: &'a PlanningState,
    pub config: SimConfig,
    /// Price/defs context for construction work and GDP guesstimates.
    pub economy: &'a EconomyContext,
    /// Follow-on / gap / rate snapshot shared by all candidates in this bag.
    pub residual: BagResidualCurr,
}

impl<'a> CheapBagCurr<'a> {
    /// Build bag context: residual snapshot on `state` plus shared sim handles.
    pub fn new(
        goal: &Goal,
        state: &'a PlanningState,
        config: SimConfig,
        economy: &'a EconomyContext,
        gdp_for_rates: f64,
    ) -> Result<Self, ProgressGapError> {
        Ok(Self {
            state,
            config,
            economy,
            residual: BagResidualCurr::compute(goal, state, config, economy, gdp_for_rates)?,
        })
    }
}

/// Rough GDP gain from finishing one ordinary building level (not Construction
/// Sector).
///
/// Walks the building’s default production methods, sums `output_qty * price`
/// (current planning price, else defs base price). **Does not** subtract
/// inputs, model staffing, or run a price/GDP solve -- emit recomputes after
/// speculative complete.
///
/// If the building is missing / yields nothing, falls back to a small fraction
/// of `|state.gdp|` so ranking still has a positive credit.
///
/// **Deficiencies vs emit / formal greedy** (keep in sync with bag-score comments):
/// - Not a full price/GDP solve.
/// - Ignores input costs, staffing, and market clearance.
/// - Follow-on days after this credit can score **lower (better)** than this
///   guesstimate implies once real slots, prices, and greedy order apply.
pub fn cheap_gdp_delta_guesstimate(
    state: &PlanningState,
    building: &str,
    economy: &EconomyContext,
) -> f64 {
    let Some(building_type) = economy.defs.building_types.get(building) else {
        return (state.gdp.abs() * 0.005).max(1.0);
    };
    // Optimistic: sum default-PM outputs * current/base price (no input subtract).
    // Mirror EconomyContext::max_output_price_over_base IO walk.
    let mut value = 0.0_f64;
    let (_, outputs) = {
        let mut inputs = vic3_defs::GoodsVec::zeros(economy.defs.goods_order.len());
        let mut outputs = vic3_defs::GoodsVec::zeros(economy.defs.goods_order.len());
        for group_id in &building_type.production_method_groups {
            let Some(pm_id) = economy
                .defs
                .production_method_groups
                .get(group_id)
                .and_then(|pms| pms.first())
                .cloned()
            else {
                continue;
            };
            let Some(pm) = economy.defs.production_methods.get(&pm_id) else {
                continue;
            };
            for (good_idx, qty) in &pm.inputs {
                inputs.add(*good_idx, *qty);
            }
            for (good_idx, qty) in &pm.outputs {
                outputs.add(*good_idx, *qty);
            }
        }
        (inputs, outputs)
    };
    for (good_idx, qty) in outputs.iter_indexed() {
        if qty <= 0.0 || !qty.is_finite() {
            continue;
        }
        let Some(good_id) = economy.defs.good_by_index(good_idx) else {
            continue;
        };
        let price = state
            .price(good_id)
            .or_else(|| economy.defs.goods.get(good_id).map(|def| def.base_price));
        let Some(price) = price.filter(|price| price.is_finite() && *price > 0.0) else {
            continue;
        };
        value += qty * price;
    }
    if value > 1e-12 {
        value
    } else {
        (state.gdp.abs() * 0.005).max(1.0)
    }
}

/// Construction points/day gained by finishing **one** Construction Sector
/// level, scaled by government construction share from laws.
///
/// Cheap analytic points delta from defs / world CS PM
/// (`country_construction_add`). Emit and greedy rebuild use the real
/// construction-point sync after the build completes instead of this estimate.
pub fn cheap_construction_sector_points_delta(
    state: &PlanningState,
    economy: &EconomyContext,
) -> f64 {
    let world = economy.apply_planning_to_world(state);

    let cs_type_id = economy.defs.building_index_of(BUILDING_CONSTRUCTION_SECTOR);

    // Prefer the live CS building’s PM on the applied world when one exists.
    let from_world_cs = world
        .buildings
        .iter()
        .find(|building| cs_type_id.is_some_and(|id| building.building_type_id == id))
        .and_then(|building| construction_add_for_cs_building(building, &economy.defs).ok());

    // Else fall back to the first CS production method’s country_construction_add in defs.
    let from_defs_pm = economy
        .defs
        .building_types
        .get(BUILDING_CONSTRUCTION_SECTOR)
        .and_then(|building_type| {
            building_type
                .production_method_groups
                .iter()
                .find_map(|group_id| {
                    let pm_ids = economy.defs.production_method_groups.get(group_id)?;
                    let pm_id = pm_ids.first()?;
                    let pm = economy.defs.production_methods.get(pm_id)?;
                    pm.country_construction_add
                        .filter(|value| value.is_finite() && *value >= 0.0)
                })
        });

    let add = from_world_cs.or(from_defs_pm).unwrap_or(0.0);
    let share = government_construction_share_from_laws(state.laws.iter().map(String::as_str));
    let delta = add * share;
    if delta.is_finite() && delta > 0.0 {
        delta
    } else {
        0.0
    }
}

/// Estimated calendar days to finish one newly queued building at the current
/// state’s construction points/day (`ceil(work / points)` when work is known;
/// otherwise falls back to construction ETA).
fn estimated_build_days(
    state: &PlanningState,
    building: &str,
    config: SimConfig,
    economy: &EconomyContext,
) -> u32 {
    let work = construction_work_points_for_enqueue(building, economy, config)
        .unwrap_or(f64::from(config.default_construction_cost));
    let points_per_day = state.construction_points_per_day.max(1e-9);
    let days = (work / points_per_day).ceil();
    if days.is_finite() && days > 0.0 {
        return (days as u32).clamp(1, u32::MAX / 4);
    }
    construction_eta_days(state, config, ConstructionEtaMode::CapacityOrSlot).max(1)
}

/// Cheap candidate-bag ranking key for one action (lower is better).
///
/// - **Construction Sector:** estimated build days + follow-on days scaled by
///   the construction-throughput ratio
///   `points_now / (points_now + ΔC)`.
///   CS also moves GDP via construction-goods demand (iron, wood, …); that
///   effect is real but secondary in this cheap path -- the primary ranking
///   benefit modeled here is shortening follow-on days via higher throughput.
///   Emit ([`emit_bag_score`]) runs a full residual on the completed world and
///   therefore sees post-complete GDP as well as throughput.
/// - **Ordinary build:** estimated build days + follow-on days after crediting
///   [`cheap_gdp_delta_guesstimate`] against the bag’s meter gap.
/// - **Other actions:** `edge_days +` follow-on residual on the current state.
///
/// Queue edges are often 0 calendar days; `build_days` stands in for completion
/// time in the ranking key while path cost still uses the real 0-then-wait
/// edges.
///
/// **Deficiencies vs emit / formal greedy rebuild** -- do not treat these
/// numbers as admissible or equal to emit:
/// - Construction Sector: scales follow-on on the **current** state by
///   throughput ratio only. Does **not** credit CS’s own GDP delta in the cheap
///   key (construction-goods demand can still move GDP; emit’s full residual
///   after speculative complete sees that). Also omits actual slots, CS finish
///   day, and a post-CS residual schedule (those appear on emit / greedy rebuild).
/// - Ordinary builds: credits a cheap GDP guesstimate, not a full price/GDP
///   solve.
/// - Bag does not re-run greedy; formal upper-bound rebuild does (and honors
///   in-flight CS).
/// - The delayed / follow-on portion can score differently than this heuristic
///   -- including **lower (better)** -- once slots, intervening completions,
///   prices, and greedy order are real. Bag score is a bias only.
///
/// Day components of bag scores are combined with [`u32::saturating_add`]
/// (also research ETA sums and [`emit_bag_score`]): ranking keys must not wrap
/// or panic on pathologically large residuals.
pub fn cheap_bag_score(action: &Action, edge_days: u16, curr: &CheapBagCurr<'_>) -> u32 {
    let residual = &curr.residual;
    match action {
        Action::QueueBuildingLevel {
            building_type_name: building,
            ..
        } if building == BUILDING_CONSTRUCTION_SECTOR => {
            cheap_construction_sector_bag_score(curr, building)
        }
        Action::QueueBuildingLevel {
            building_type_name: building,
            ..
        } => {
            let build_days = estimated_build_days(curr.state, building, curr.config, curr.economy);
            // Only credit a GDP guesstimate against GDP-only open goals; otherwise
            // keep full follow-on (tech/law/etc. must not be erased by wood output).
            let follow = if residual.allow_cheap_gdp_credit {
                let cheap_delta = cheap_gdp_delta_guesstimate(curr.state, building, curr.economy);
                let residual_gap = (residual.meter_gap - cheap_delta).max(0.0);
                if residual.aggregate_rate > 1e-12 {
                    let days = (residual_gap / residual.aggregate_rate).ceil();
                    if days.is_finite() && days > 0.0 {
                        (days as u32).min(u32::MAX / 4)
                    } else {
                        0
                    }
                } else {
                    residual.follow_on_days
                }
            } else {
                residual.follow_on_days
            };
            // Queue edges are 0-day; build_days stands in for completion time in
            // the ranking key (path cost still uses the real 0 then wait).
            let _ = edge_days;
            build_days.saturating_add(follow)
        }
        _ => u32::from(edge_days).saturating_add(residual.follow_on_days),
    }
}

/// Construction Sector cheap score: build time + scaled follow-on days.
///
/// `follow ≈ points_now / (points_now + points_delta) * follow_on_days` when
/// throughput is already positive. Primary cheap benefit is throughput; GDP
/// from construction-goods demand is left to emit’s full residual. Not equal
/// to emit/rebuild (those use real slots and post-CS residual).
fn cheap_construction_sector_bag_score(curr: &CheapBagCurr<'_>, building: &str) -> u32 {
    let follow_on_days = curr.residual.follow_on_days;
    let build_days = estimated_build_days(curr.state, building, curr.config, curr.economy);
    let points_now = curr.state.construction_points_per_day;
    let points_delta = cheap_construction_sector_points_delta(curr.state, curr.economy);

    // Construction-unit scale of follow-on days on state_t:
    //   follow ≈ points_now / (points_now + points_delta) * follow_on_days
    // when points_now > 0. Not equal to emit/rebuild (actual slots + CS finish).
    let follow = if points_now > 1e-12 && points_delta > 1e-12 {
        let scale = points_now / (points_now + points_delta);
        let scaled = (scale * f64::from(follow_on_days)).ceil();
        if scaled.is_finite() && scaled >= 0.0 {
            (scaled as u32).min(u32::MAX / 4)
        } else {
            follow_on_days
        }
    } else if points_delta > 1e-12 {
        // First CS: no ratio; keep follow-on on state_t as a coarse stand-in.
        follow_on_days
    } else {
        follow_on_days
    };
    build_days.saturating_add(follow)
}

/// Emit-time ranking key after building a **speculatively completed** planning
/// world: `edge_days +` residual days on that completed state.
///
/// Unlike [`cheap_bag_score`], this runs the full
/// [`rank_heuristic_with_gdp_for_rates`] on `completed_state` (so a finished
/// Construction Sector sees real post-complete throughput **and** GDP,
/// including construction-goods demand). It does **not** use the cheap
/// construction-unit scale.
///
/// Day components are combined with [`u32::saturating_add`] (same policy as
/// [`cheap_bag_score`] and research ETA sums): ranking keys must not wrap or
/// panic if residuals are pathologically large.
pub fn emit_bag_score(
    edge_days: u16,
    goal: &Goal,
    completed_state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
) -> Result<u32, ProgressGapError> {
    let residual = rank_heuristic_with_gdp_for_rates(
        goal,
        completed_state,
        config,
        economy,
        completed_state.gdp,
    )?;
    Ok(u32::from(edge_days).saturating_add(residual))
}

/// How much `after` improved `subgoal` relative to `before`.
///
/// `max(0, gap(before) - gap(after))`. Bubbles [`ProgressGapError`] if either
/// gap cannot be evaluated.
pub fn simple_subgoal_delta(
    subgoal: &SimpleSubgoal,
    before: &PlanningState,
    after: &PlanningState,
) -> Result<f64, ProgressGapError> {
    let gap_before = simple_subgoal_gap(subgoal, before)?;
    let gap_after = simple_subgoal_gap(subgoal, after)?;
    Ok((gap_before - gap_after).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::test_support::{ger_state, logging_and_cs_economy};
    use crate::world::{PlanningParts, PlanningState};

    /// GER planning state with GDP 1000, 5 construction points/day, wood @ 40.
    fn state_curr_for_cheap_bag() -> PlanningState {
        ger_state().gdp(1000.0).points(5.0).wood_price(40.0).get()
    }

    /// Logging+CS mini economy with logging `required_construction` overridden.
    fn logging_economy_with_cost(logging_required_construction: f64) -> EconomyContext {
        logging_and_cs_economy()
            .building_cost("building_logging_camp", logging_required_construction)
            .economy
    }

    /// Larger GDP shortfall should not rank *better* (lower) than a smaller one.
    #[test]
    fn gdp_gap_and_rank_increase_with_shortfall() {
        let state = PlanningState::from_parts(PlanningParts {
            gdp: 1000.0,
            ..PlanningParts::default()
        });
        let economy = EconomyContext::empty();
        let small = compile("gdp >= 1100").unwrap();
        let big = compile("gdp >= 5000").unwrap();
        let h_small = rank_heuristic(&small, &state, SimConfig::default(), &economy).unwrap();
        let h_big = rank_heuristic(&big, &state, SimConfig::default(), &economy).unwrap();
        assert!(h_big >= h_small, "h_big={h_big} h_small={h_small}");
        assert!(h_small >= 1);
    }

    /// Satisfied GDP goal ranks as zero residual days.
    #[test]
    fn cleared_gdp_ranks_zero() {
        let state = PlanningState::from_parts(PlanningParts {
            gdp: 2000.0,
            ..PlanningParts::default()
        });
        let goal = compile("gdp >= 1000").unwrap();
        assert_eq!(
            rank_heuristic(
                &goal,
                &state,
                SimConfig::default(),
                &EconomyContext::empty()
            )
            .unwrap(),
            0
        );
    }

    /// Missing price meter must error (no silent "already satisfied" gap).
    #[test]
    fn missing_good_price_gap_errors() {
        let state = PlanningState::from_parts(PlanningParts {
            gdp: 1000.0,
            ..PlanningParts::default()
        });
        let subgoal = SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 30.0,
        };
        let err = simple_subgoal_gap(&subgoal, &state).unwrap_err();
        assert_eq!(
            err,
            ProgressGapError::MissingMeter {
                meter: "good price"
            }
        );
    }

    /// Fixture CS PM (`country_construction_add` 5) * government share 1.0 -> +5 points/day.
    #[test]
    fn cheap_cs_points_delta_from_economy_fixture() {
        let mini = logging_and_cs_economy();
        let state_curr = ger_state().gdp(1000.0).points(5.0).wood_price(30.0).get();
        let points_delta = cheap_construction_sector_points_delta(&state_curr, &mini.economy);
        assert!(
            (points_delta - 5.0).abs() < 1e-9,
            "expected government share 1.0 * country_construction_add 5 -> 5, got {points_delta}"
        );
    }

    /// When CS points delta equals current throughput, cheap follow-on scales by ~1/2.
    ///
    /// With points_now = 5 and delta = 5, follow ~= ceil(5/(5+5) * 100) = 50 for an
    /// overridden follow_on_days of 100 -- the throughput-ratio path in
    /// [`cheap_construction_sector_bag_score`].
    #[test]
    fn cheap_cs_bag_score_halves_follow_on_when_delta_equals_points() {
        let mini = logging_and_cs_economy();
        let config = SimConfig::default();
        let state_curr = ger_state().gdp(1000.0).points(5.0).wood_price(30.0).get();
        let goal = compile("gdp >= 5000").unwrap();
        let action = Action::QueueBuildingLevel {
            building_type_name: BUILDING_CONSTRUCTION_SECTOR.into(),
            state_id: 1,
        };

        let work_points = construction_work_points_for_enqueue(
            BUILDING_CONSTRUCTION_SECTOR,
            &mini.economy,
            config,
        )
        .expect("CS required_construction");
        let points_per_day = state_curr.construction_points_per_day.max(1e-9);
        let estimated_build_days = (work_points / points_per_day).ceil() as u32;
        let expected_follow_on = 50_u32;

        let mut curr = CheapBagCurr::new(&goal, &state_curr, config, &mini.economy, state_curr.gdp)
            .expect("bag residual");
        curr.residual.follow_on_days = 100;
        let score = cheap_bag_score(&action, 0, &curr);
        assert_eq!(
            score,
            estimated_build_days + expected_follow_on,
            "score={score} build_days={estimated_build_days} follow_portion should be 50"
        );
        assert_eq!(score - estimated_build_days, expected_follow_on);
    }

    /// Logging camp cheap GDP guesstimate is output qty * wood price (10 * 40 = 400).
    #[test]
    fn cheap_gdp_delta_guesstimate_positive_for_logging() {
        let economy = logging_economy_with_cost(30.0);
        let state_curr = state_curr_for_cheap_bag();
        let delta = cheap_gdp_delta_guesstimate(&state_curr, "building_logging_camp", &economy);
        assert!(
            delta > 0.0,
            "expected positive wood*price guesstimate, got {delta}"
        );
        // 10 wood * price 40
        assert!(
            (delta - 400.0).abs() < 1e-9,
            "expected 10*40=400, got {delta}"
        );
    }

    /// Ordinary-build cheap score must shrink follow-on after GDP credit vs no-credit baseline.
    #[test]
    fn cheap_build_bag_score_credits_gdp_guesstimate() {
        let economy = logging_economy_with_cost(30.0);
        let state_curr = state_curr_for_cheap_bag();
        let config = SimConfig::default();
        let action = Action::QueueBuildingLevel {
            building_type_name: "building_logging_camp".into(),
            state_id: 1,
        };
        // Without GDP credit: residual_gap/rate = 1000/10 -> follow 100.
        let residual = BagResidualCurr {
            follow_on_days: 200,
            meter_gap: 1000.0,
            aggregate_rate: 10.0,
            allow_cheap_gdp_credit: true,
        };
        let cheap_delta =
            cheap_gdp_delta_guesstimate(&state_curr, "building_logging_camp", &economy);
        assert!(cheap_delta > 0.0);

        let curr = CheapBagCurr {
            state: &state_curr,
            config,
            economy: &economy,
            residual,
        };
        let score = cheap_bag_score(&action, 0, &curr);
        let work = construction_work_points_for_enqueue("building_logging_camp", &economy, config)
            .expect("logging required_construction");
        let build_days = (work / state_curr.construction_points_per_day.max(1e-9)).ceil() as u32;
        let follow_portion = score - build_days;
        let residual_without_credit = (residual.meter_gap / residual.aggregate_rate).ceil() as u32;
        assert_eq!(residual_without_credit, 100);
        assert!(
            follow_portion < 100,
            "follow portion {follow_portion} should be < 100 after crediting cheap_delta={cheap_delta}"
        );
    }

    /// Very expensive logging should lose to CS when CS halves a large follow-on residual.
    #[test]
    fn cheap_cs_can_outrank_slow_productive_build() {
        // Huge logging construction -> build_days dominate; CS scales follow-on by 0.5.
        let economy = logging_economy_with_cost(100_000.0);
        let state_curr = state_curr_for_cheap_bag();
        let config = SimConfig::default();
        let curr = CheapBagCurr {
            state: &state_curr,
            config,
            economy: &economy,
            residual: BagResidualCurr {
                follow_on_days: 10_000,
                meter_gap: 4000.0,
                aggregate_rate: 10.0,
                allow_cheap_gdp_credit: true,
            },
        };

        let cs_action = Action::QueueBuildingLevel {
            building_type_name: BUILDING_CONSTRUCTION_SECTOR.into(),
            state_id: 1,
        };
        let logging_action = Action::QueueBuildingLevel {
            building_type_name: "building_logging_camp".into(),
            state_id: 1,
        };

        let cs_score = cheap_bag_score(&cs_action, 0, &curr);
        let logging_score = cheap_bag_score(&logging_action, 0, &curr);
        assert!(
            cs_score < logging_score,
            "CS speedup should outrank slow logging: cs={cs_score} logging={logging_score}"
        );
    }

    /// Emit ranking must use completed-world GDP residual, not the pre-complete gap.
    #[test]
    fn emit_bag_score_uses_completed_gdp_not_curr_gap() {
        let goal = compile("gdp >= 5000").unwrap();
        let config = SimConfig::default();
        let economy = EconomyContext::empty();
        let edge_days: u16 = 30;

        let state_curr = PlanningState::from_parts(PlanningParts {
            gdp: 1000.0,
            ..PlanningParts::default()
        });
        let completed = PlanningState::from_parts(PlanningParts {
            gdp: 4800.0,
            ..PlanningParts::default()
        });

        let emit = emit_bag_score(edge_days, &goal, &completed, config, &economy).unwrap();
        let naive_curr = u32::from(edge_days)
            .saturating_add(rank_heuristic(&goal, &state_curr, config, &economy).unwrap());
        let residual_completed =
            rank_heuristic_with_gdp_for_rates(&goal, &completed, config, &economy, completed.gdp)
                .unwrap();

        assert!(
            emit < naive_curr,
            "emit={emit} should beat edge+curr residual={naive_curr}"
        );
        assert_eq!(
            emit,
            u32::from(edge_days).saturating_add(residual_completed),
            "emit must be edge + residual on completed GDP"
        );
        // Current state still has a large open gap; completed nearly clears it.
        assert!(rank_heuristic(&goal, &state_curr, config, &economy).unwrap() > residual_completed);
    }
}
