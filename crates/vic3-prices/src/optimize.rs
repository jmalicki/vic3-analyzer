//! Greedy production-method search over player buildings.
//!
//! Candidates are PMs already used by the same building type in the world.
//! Each trial replaces one PM slot on a type-aggregated clone and re-solves
//! via [`crate::preview`] with a warm start. Heuristic only — see
//! [`PM_SEARCH_HEURISTIC`] / [`MAX_PM_TRIALS`].

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vic3_defs::GameDefs;

use crate::result::{PricesResult, ProductionMethodDelta, SolveOpts, SolveStatus, WorldDelta};
use crate::world::{World, WorldBuilding, WorldCountry};
use crate::{preview, LIMITATIONS};

/// Trial budget for [`optimize_pms`] (each trial is one [`preview`] solve).
pub const MAX_PM_TRIALS: u32 = 40;

/// Appended when a country lists researched techs but defs have no unlock table.
pub const PM_TECH_GATING_INCOMPLETE: &str = "PM tech gating incomplete";

/// Appended so callers do not treat the greedy walk as exhaustive.
pub const PM_SEARCH_HEURISTIC: &str =
    "PM search is a greedy one-slot heuristic, not a full combinatorial search.";

const SCORE_EPS: f64 = 1e-9;

/// Objective for [`optimize_pms`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptimizeAxis {
    Income,
    Productivity,
    Sol,
}

/// JSON body for [`optimize_pms`] / wasm `loaded_optimize_pms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptimizePmsOpts {
    pub axis: OptimizeAxis,
}

/// One suggested production-method swap on a building instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OptimizeChange {
    pub building_type: String,
    pub building_id: u32,
    pub from: Vec<String>,
    pub to: Vec<String>,
}

/// Score deltas versus the baseline solve (`new − old`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OptimizeDelta {
    pub income: f64,
    pub productivity: f64,
    pub sol: f64,
    pub residual: f64,
}

/// Compact numbers from the last trial solve. Full [`PricesResult`] is omitted
/// because a late-game dump is huge; these are the values the UI needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OptimizePricesSummary {
    pub residual: f64,
    pub status: SolveStatus,
    pub income: f64,
    pub productivity: f64,
    pub sol: f64,
}

/// Suggested PM changes, estimated Δ, and a [`WorldDelta`] for a later apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OptimizeResult {
    pub axis: OptimizeAxis,
    pub changes: Vec<OptimizeChange>,
    pub delta: OptimizeDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prices: Option<OptimizePricesSummary>,
    pub limitations: Vec<String>,
    pub world_delta: WorldDelta,
}

#[derive(Clone, Copy)]
struct Scores {
    income: f64,
    productivity: f64,
    sol: f64,
    residual: f64,
}

/// Suggest production-method changes that improve `axis` for player buildings.
///
/// `world` is not mutated. Each trial calls [`preview`] with `opts.warm_rel`
/// taken from `baseline.relative` when that vector is non-empty.
///
/// # Arguments
///
/// * `world` / `defs` — same as [`crate::solve`].
/// * `baseline` — prior solve on `world` (scores + warm_rel source).
/// * `opts` — residual / iteration budget for each trial preview.
/// * `axis` — [`OptimizeAxis`] objective.
///
/// # Returns
///
/// [`OptimizeResult`] with suggested changes, score deltas, a compact prices
/// summary, `limitations` (crate list + heuristic / tech-gating notes), and a
/// [`WorldDelta`] suitable for a later apply. No Rust `Err`.
pub fn optimize_pms(
    world: &World,
    defs: &GameDefs,
    baseline: &PricesResult,
    mut opts: SolveOpts,
    axis: OptimizeAxis,
) -> OptimizeResult {
    if !baseline.relative.is_empty() {
        opts.warm_rel = Some(baseline.relative.clone());
    }

    let player_states = player_state_ids(world);
    let player_ids = player_building_ids(world, player_states.as_ref());
    let mut limitations = baseline.limitations.clone();
    if limitations.is_empty() {
        limitations = LIMITATIONS.iter().map(|line| (*line).to_string()).collect();
    }
    limitations.push(PM_SEARCH_HEURISTIC.to_string());
    if techs_present(world) {
        limitations.push(PM_TECH_GATING_INCOMPLETE.to_string());
    }

    let baseline_scores = scores_from_result(world, baseline, &player_ids, player_states.as_ref());
    let mut best_scores = baseline_scores;
    let mut accepted = WorldDelta::default();
    let mut last_prices: Option<OptimizePricesSummary> = None;
    let mut trials = 0u32;
    let ctx = TrialCtx {
        world,
        defs,
        opts: &opts,
        player_ids: &player_ids,
        player_states: player_states.as_ref(),
        axis,
    };

    let mut by_type: BTreeMap<String, Vec<&WorldBuilding>> = BTreeMap::new();
    for building in &world.buildings {
        if player_ids.contains(&building.id) {
            by_type
                .entry(building.type_script_id(defs).to_string())
                .or_default()
                .push(building);
        }
    }

    for (type_id, buildings) in by_type {
        if trials >= MAX_PM_TRIALS {
            break;
        }
        let candidates = candidate_pms(world, defs, &type_id);
        if candidates.is_empty() {
            continue;
        }
        let mut current_methods = buildings
            .iter()
            .max_by_key(|building| {
                (
                    building.production_methods.len(),
                    std::cmp::Reverse(building.id),
                )
            })
            .map(|building| building.production_methods.clone())
            .unwrap_or_default();
        if current_methods.is_empty() {
            continue;
        }

        let needs_unify = buildings
            .iter()
            .any(|building| building.production_methods != current_methods);
        if needs_unify {
            if let Some(outcome) = try_type_methods(
                &ctx,
                &accepted,
                &buildings,
                &current_methods,
                best_scores.for_axis(axis),
                &mut trials,
            ) {
                accepted = outcome.delta;
                best_scores = outcome.scores;
                last_prices = Some(outcome.summary);
            }
        }

        let slot_count = current_methods.len();
        let mut improved = true;
        while improved && trials < MAX_PM_TRIALS {
            improved = false;
            'slots: for slot in 0..slot_count {
                for candidate in &candidates {
                    if current_methods
                        .get(slot)
                        .is_some_and(|current| current == candidate)
                    {
                        continue;
                    }
                    let mut trial_methods = current_methods.clone();
                    trial_methods[slot] = candidate.clone();
                    if let Some(outcome) = try_type_methods(
                        &ctx,
                        &accepted,
                        &buildings,
                        &trial_methods,
                        best_scores.for_axis(axis),
                        &mut trials,
                    ) {
                        accepted = outcome.delta;
                        best_scores = outcome.scores;
                        last_prices = Some(outcome.summary);
                        current_methods = trial_methods;
                        improved = true;
                        break 'slots;
                    }
                    if trials >= MAX_PM_TRIALS {
                        break 'slots;
                    }
                }
            }
        }
    }

    let changes = changes_from_delta(world, defs, &accepted);
    OptimizeResult {
        axis,
        changes,
        delta: OptimizeDelta {
            income: best_scores.income - baseline_scores.income,
            productivity: best_scores.productivity - baseline_scores.productivity,
            sol: best_scores.sol - baseline_scores.sol,
            residual: best_scores.residual - baseline_scores.residual,
        },
        prices: last_prices.or(Some(OptimizePricesSummary {
            residual: baseline.residual,
            status: baseline.status,
            income: baseline_scores.income,
            productivity: baseline_scores.productivity,
            sol: baseline_scores.sol,
        })),
        limitations,
        world_delta: accepted,
    }
}

struct TrialCtx<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    opts: &'a SolveOpts,
    player_ids: &'a BTreeSet<u32>,
    player_states: Option<&'a BTreeSet<u32>>,
    axis: OptimizeAxis,
}

struct TrialOutcome {
    delta: WorldDelta,
    scores: Scores,
    summary: OptimizePricesSummary,
}

fn try_type_methods(
    ctx: &TrialCtx<'_>,
    accepted: &WorldDelta,
    buildings: &[&WorldBuilding],
    methods: &[String],
    best_axis: f64,
    trials: &mut u32,
) -> Option<TrialOutcome> {
    if *trials >= MAX_PM_TRIALS {
        return None;
    }
    let mut delta = accepted.clone();
    let mut changed = false;
    for building in buildings {
        let already = delta
            .production_methods
            .iter()
            .find(|entry| entry.building_id == building.id)
            .map(|entry| entry.methods.as_slice())
            .unwrap_or(building.production_methods.as_slice());
        if already == methods {
            continue;
        }
        delta
            .production_methods
            .retain(|entry| entry.building_id != building.id);
        delta.production_methods.push(ProductionMethodDelta {
            building_id: building.id,
            methods: methods.to_vec(),
        });
        changed = true;
    }
    if !changed {
        return None;
    }
    *trials += 1;
    let result = preview(ctx.world, ctx.defs, &delta, ctx.opts.clone());
    let scores = scores_from_result(ctx.world, &result, ctx.player_ids, ctx.player_states);
    if scores.for_axis(ctx.axis) <= best_axis + SCORE_EPS {
        return None;
    }
    Some(TrialOutcome {
        summary: OptimizePricesSummary {
            residual: result.residual,
            status: result.status,
            income: scores.income,
            productivity: scores.productivity,
            sol: scores.sol,
        },
        delta,
        scores,
    })
}

fn candidate_pms(world: &World, defs: &GameDefs, building_type: &str) -> Vec<String> {
    let want = defs.building_index_of(building_type);
    let mut ids = BTreeSet::new();
    for building in &world.buildings {
        if Some(building.building_type_id) == want {
            ids.extend(building.production_methods.iter().cloned());
        }
    }
    ids.into_iter().collect()
}

fn player_state_ids(world: &World) -> Option<BTreeSet<u32>> {
    let country = player_country(world)?;
    let mut ids: BTreeSet<u32> = country.states.iter().copied().collect();
    for state in &world.states {
        if state.country == Some(country.id) {
            ids.insert(state.id);
        }
    }
    Some(ids)
}

fn player_country(world: &World) -> Option<&WorldCountry> {
    world
        .player_country_tag()
        .and_then(|tag| world.country_by_tag(tag))
}

fn player_building_ids(world: &World, player_states: Option<&BTreeSet<u32>>) -> BTreeSet<u32> {
    match player_states {
        None => world.buildings.iter().map(|building| building.id).collect(),
        Some(states) => world
            .buildings
            .iter()
            .filter(|building| building.state.is_some_and(|state| states.contains(&state)))
            .map(|building| building.id)
            .collect(),
    }
}

fn techs_present(world: &World) -> bool {
    if let Some(country) = player_country(world) {
        return !country.techs.is_empty();
    }
    world
        .countries
        .iter()
        .any(|country| !country.techs.is_empty())
}

fn scores_from_result(
    world: &World,
    result: &PricesResult,
    player_ids: &BTreeSet<u32>,
    player_states: Option<&BTreeSet<u32>>,
) -> Scores {
    let mut income = 0.0;
    let mut staffed = 0.0;
    for building in &result.buildings {
        if !player_ids.contains(&building.id) {
            continue;
        }
        income += building.profit;
        staffed += building.staffing.clamp(0.0, building.level.max(0.0));
    }
    let productivity = if staffed > 0.0 { income / staffed } else { 0.0 };
    Scores {
        income,
        productivity,
        sol: mean_wealth(world, result, player_states),
        residual: result.residual,
    }
}

fn mean_wealth(world: &World, result: &PricesResult, player_states: Option<&BTreeSet<u32>>) -> f64 {
    let mut total = 0.0;
    let mut count = 0.0;
    if !result.state_pops.is_empty() {
        for pop in result.state_pops.iter() {
            if !pop_in_scope(pop.state_id, player_states) {
                continue;
            }
            if let Some(wealth) = pop.wealth {
                total += f64::from(wealth);
                count += 1.0;
            }
        }
    } else {
        for pop in &world.state_pops {
            let Some(state) = pop.state else {
                continue;
            };
            if !pop_in_scope(state, player_states) {
                continue;
            }
            if let Some(wealth) = pop.wealth {
                total += f64::from(wealth);
                count += 1.0;
            }
        }
        if count == 0.0 {
            for pop in &world.pops {
                total += f64::from(pop.wealth);
                count += 1.0;
            }
        }
    }
    if count > 0.0 {
        total / count
    } else {
        0.0
    }
}

fn pop_in_scope(state_id: u32, player_states: Option<&BTreeSet<u32>>) -> bool {
    player_states.is_none_or(|states| states.contains(&state_id))
}

impl Scores {
    fn for_axis(&self, axis: OptimizeAxis) -> f64 {
        match axis {
            OptimizeAxis::Income => self.income,
            OptimizeAxis::Productivity => self.productivity,
            OptimizeAxis::Sol => self.sol,
        }
    }
}

fn changes_from_delta(world: &World, defs: &GameDefs, delta: &WorldDelta) -> Vec<OptimizeChange> {
    let mut changes = Vec::new();
    for entry in &delta.production_methods {
        let Some(building) = world
            .buildings
            .iter()
            .find(|building| building.id == entry.building_id)
        else {
            continue;
        };
        if building.production_methods == entry.methods {
            continue;
        }
        changes.push(OptimizeChange {
            building_type: building.type_script_id(defs).to_string(),
            building_id: building.id,
            from: building.production_methods.clone(),
            to: entry.methods.clone(),
        });
    }
    changes.sort_by(|left, right| {
        left.building_type
            .cmp(&right.building_type)
            .then(left.building_id.cmp(&right.building_id))
    });
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use vic3_defs::{Good, GoodId, ProductionMethod};

    use crate::solve;
    use crate::world::WorldBuilding;

    fn grain_defs() -> GameDefs {
        let mut defs = GameDefs {
            price_range: 0.75,
            goods_order: vec!["grain".into()],
            ..GameDefs::default()
        };
        defs.goods.insert(
            "grain".into(),
            Good {
                name: "grain".into(),
                base_price: 20.0,
                traded_quantity: 12.0,
                texture: None,
            },
        );
        defs.production_methods.insert(
            "pm_rich".into(),
            ProductionMethod {
                name: "pm_rich".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodId::from_usize(0), 40.0)],
                ..Default::default()
            },
        );
        defs.production_methods.insert(
            "pm_poor".into(),
            ProductionMethod {
                name: "pm_poor".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodId::from_usize(0), 1.0)],
                ..Default::default()
            },
        );
        defs.ensure_building_type("workshop");
        defs
    }

    fn workshop(defs: &GameDefs, id: u32, method: &str) -> WorldBuilding {
        WorldBuilding {
            id,
            state: None,
            building_type_id: defs.building_index_of("workshop").unwrap(),
            level: 1.0,
            staffing: 1.0,
            production_methods: vec![method.into()],
            saved_inputs: Vec::new(),
            saved_outputs: Vec::new(),
        }
    }

    #[test]
    fn optimizer_selects_strictly_more_profitable_pm() {
        let defs = grain_defs();
        let world = World {
            buildings: vec![workshop(&defs, 1, "pm_rich"), workshop(&defs, 2, "pm_poor")],
            ..World::default()
        };
        let baseline = solve(&world, &defs, SolveOpts::default());
        let result = optimize_pms(
            &world,
            &defs,
            &baseline,
            SolveOpts::default(),
            OptimizeAxis::Income,
        );

        let poor = result
            .changes
            .iter()
            .find(|change| change.building_id == 2)
            .expect("poor workshop should change");
        assert_eq!(poor.building_type, "workshop");
        assert_eq!(poor.from, ["pm_poor"]);
        assert_eq!(poor.to, ["pm_rich"]);
        assert!(
            result.delta.income > 0.0,
            "income should rise, got {}",
            result.delta.income
        );
        assert_eq!(
            result.world_delta.production_methods[0].methods,
            ["pm_rich"]
        );
        assert!(result
            .limitations
            .iter()
            .any(|line| line == PM_SEARCH_HEURISTIC));
    }

    #[test]
    fn optimizer_adds_tech_gating_limitation_when_techs_present() {
        let defs = grain_defs();
        let world = World {
            player_tag: Some("TST".into()),
            countries: vec![WorldCountry {
                tag: "TST".into(),
                techs: vec!["tech_romanticism".into()],
                ..WorldCountry::default()
            }],
            buildings: vec![workshop(&defs, 1, "pm_poor")],
            ..World::default()
        };
        let baseline = solve(&world, &defs, SolveOpts::default());
        let result = optimize_pms(
            &world,
            &defs,
            &baseline,
            SolveOpts::default(),
            OptimizeAxis::Productivity,
        );
        assert!(result
            .limitations
            .iter()
            .any(|line| line == PM_TECH_GATING_INCOMPLETE));
    }
}
