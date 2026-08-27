//! Goal-relevant simulator successors over [`crate::world::PlanningState`].
//!
//! # Why goal-relevant only
//!
//! Enumerating the whole game is intractable. [`successors`] / [`successors_for_simple_subgoals`]
//! open only edges that can close currently failing [`crate::goals::SimpleSubgoal`]s (plus
//! building / PM candidates from [`EconomyContext`]). Idle
//! atoms with no model action emit nothing.
//!
//! Building candidates are `(building_type, state_id)` for [`Action::QueueBuildingLevel`]:
//! defs-based goal-relevant types (first-of-type allowed), expanded to Vic3
//! placement states (existing instances, else every owned state), plus
//! Construction Sector as a meta lever. No IO “dominance” prune — modeled
//! axes omit slots, local markets, and unlock gates, so type-only dominance
//! is not sound.
//!
//! // TODO(anytime-ub): wide candidate sets are acceptable without top-N
//! // ranking; a later PR can seed a greedy feasible path as incumbent `U`
//! // and prune search nodes with `g + h >= U`.
//!
//! // TODO(buildability): gate placements on unlock techs + free slots /
//! // potentials before offering a state.
//!
//! # Edges
//!
//! - **Decision** (0 days): queue tech/building/interest/army/law, `SwitchPm`,
//!   `AdjustTax`. Each track has its own occupancy check (research vs
//!   construction may run in parallel). Instant decisions are not blocked by
//!   other tracks' waits.
//! - **At most one event-wait** per expansion: the earliest completion among
//!   in-flight tracks that open atoms still need (including finishing an
//!   unrelated save queue that blocks the track), or payday. Other tracks'
//!   timers tick in parallel via [`PlanningState::tick_parallel_tracks`].
//!   **I6:** wait never decreases date.
//!
//! Independent tracks ⇒ AND goals can finish near `max` rather than `sum` of
//! waits when work overlaps.
//!
//! # Invariants
//!
//! - **I6** — monotone dates; no spurious waits (property-tested).
//! - **I8** — [`apply_action`] on identical state is deterministic / same fingerprint.
//!
//! This module does **not** search; `crate::plan` owns A*.
//! See [`docs/planning.md`](../../../docs/planning.md).

use crate::construction::{
    construction_points_per_day_per_job, construction_queue_full, construction_wait_days,
    construction_wait_target, construction_work_complete, construction_work_points_for_enqueue,
    ensure_construction_work_points, maybe_add_construction_sector_candidate,
    sync_construction_points_per_day, BUILDING_CONSTRUCTION_SECTOR,
};
use crate::goals::{gaps, Goal, InterestKind, Rel, SimpleSubgoal};
use crate::military::{
    is_barracks_building, is_military_planning_building, is_naval_admin_building,
    is_shipyard_building, UnitCombatStats, BUILDING_BARRACKS, BUILDING_NAVAL_ADMIN,
    BUILDING_SHIPYARD, MIL_INPUT_PRICE_FACTOR,
};
use crate::world::{PlanningState, QueuedInterest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;
use vic3_defs::{BuildingType, GameDefs};
use vic3_prices::{
    equilibrate_cached, ShopCache, SolveOpts, SolveOutcome, World, WorldBuilding, ORDER_EPS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MilitaryBranch {
    Army,
    Navy,
}

/// Grouped arguments for military power-projection successor helpers
/// ([`push_military_pp_decisions`], [`push_mil_hire_decisions`]).
struct MilitaryPpDecisionArgs<'a> {
    result: &'a mut Vec<Successor>,
    state: &'a PlanningState,
    economy: &'a EconomyContext,
    config: SimConfig,
    seen_hires: &'a mut BTreeSet<String>,
}

fn push_military_pp_decisions(
    args: &mut MilitaryPpDecisionArgs<'_>,
    branch: MilitaryBranch,
    rel: Rel,
    value: f64,
) {
    let current = match branch {
        MilitaryBranch::Army => args.state.army_power_projection,
        MilitaryBranch::Navy => args.state.navy_power_projection,
    };
    let Some(needed) = power_raise_needed(rel, value, current) else {
        // Still may need to hire underemployed buildings so the simple subgoal can clear.
        let underemployed = match branch {
            MilitaryBranch::Army => !args.state.army_buildings_fully_staffed(),
            MilitaryBranch::Navy => !args.state.navy_buildings_fully_staffed(),
        };
        if underemployed {
            push_mil_hire_decisions(args, branch);
        }
        return;
    };
    let staffed = match branch {
        MilitaryBranch::Army => args.state.army_buildings_fully_staffed(),
        MilitaryBranch::Navy => args.state.navy_buildings_fully_staffed(),
    };
    if needed <= 0.0 && staffed {
        return;
    }
    // Prefer staffing existing underemployed capacity before building more.
    // Barracks / shipyards / naval admin levels are queued via
    // [`EconomyContext::building_candidates`] like any other building.
    let _ = push_mil_hire_decisions(args, branch);
}

fn mil_levels(state: &PlanningState, pred: fn(&str) -> bool) -> u32 {
    state
        .mil_buildings
        .iter()
        .filter(|b| pred(&b.building))
        .map(|b| b.levels.floor() as u32)
        .sum()
}

/// Default production methods: first PM id in each production-method group.
fn default_pms_for_building(defs: &GameDefs, building_type: &BuildingType) -> Vec<String> {
    building_type
        .production_method_groups
        .iter()
        .filter_map(|group_id| {
            defs.production_method_groups
                .get(group_id)
                .and_then(|pms| pms.first())
                .cloned()
        })
        .collect()
}

/// Per-level IO from default PMs (scale = 1.0 staffed level).
fn default_building_io_per_level(
    defs: &GameDefs,
    building_type: &BuildingType,
) -> (vic3_defs::GoodsVec, vic3_defs::GoodsVec) {
    let mut inputs = vic3_defs::GoodsVec::zeros(defs.goods_order.len());
    let mut outputs = vic3_defs::GoodsVec::zeros(defs.goods_order.len());
    for pm_id in default_pms_for_building(defs, building_type) {
        let Some(pm) = defs.production_methods.get(&pm_id) else {
            continue;
        };
        for (good, qty) in &pm.inputs {
            inputs.add(*good, *qty);
        }
        for (good, qty) in &pm.outputs {
            outputs.add(*good, *qty);
        }
    }
    (inputs, outputs)
}

/// Synthetic fully-staffed row for a type absent from the base world (greenfield).
fn synthetic_world_building(
    defs: &GameDefs,
    world: &World,
    building: &str,
    state_id: u32,
    levels: u32,
) -> Option<WorldBuilding> {
    let building_type = defs.buildings.get(building)?;
    let level = f64::from(levels);
    if level <= 0.0 {
        return None;
    }
    let next_id = world
        .buildings
        .iter()
        .map(|row| row.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    Some(WorldBuilding {
        id: next_id,
        // TODO(buildability): verify free slot / potential in this state.
        state: Some(state_id),
        building: building.to_string(),
        level,
        staffing: level,
        production_methods: default_pms_for_building(defs, building_type),
        saved_inputs: Vec::new(),
        saved_outputs: Vec::new(),
    })
}

/// Queue hire for underemployed buildings on this branch. Returns true if any hire was pushed.
fn push_mil_hire_decisions(args: &mut MilitaryPpDecisionArgs<'_>, branch: MilitaryBranch) -> bool {
    let MilitaryPpDecisionArgs {
        result,
        state,
        economy,
        config,
        seen_hires,
        ..
    } = args;
    let mut pushed = false;
    for row in &state.mil_buildings {
        if row.is_fully_staffed() {
            continue;
        }
        let relevant = match branch {
            MilitaryBranch::Army => is_barracks_building(&row.building),
            MilitaryBranch::Navy => {
                is_shipyard_building(&row.building) || is_naval_admin_building(&row.building)
            }
        };
        if !relevant || !seen_hires.insert(row.building.clone()) {
            continue;
        }
        push_decision(
            result,
            state,
            Action::QueueHireMilitary {
                building: row.building.clone(),
            },
            economy,
            *config,
        );
        pushed = true;
    }
    pushed
}

/// Immutable price-solver inputs shared by all nodes in one search.
///
/// Built once per search and shared (cheap `Clone` via [`Arc`] on shops).
/// Search only applies transitions; shop patching + NLS live inside apply.
///
/// [`Self::defs`] also supplies building `required_construction` when the sim
/// enqueues a new level (`QueueBuildingLevel`); in-flight save `remaining`
/// still wins for ETA.
///
/// **Baseline shops** (`shop_cache`) are derived from [`Self::base_world`] at
/// construction. They are **not** part of A* identity — only planning deltas on
/// the node are.
#[derive(Debug, Clone)]
pub struct EconomyContext {
    pub base_world: World,
    pub defs: GameDefs,
    pub solve_opts: SolveOpts,
    /// Frozen settle inputs for the unmodified base world (shared).
    pub shop_cache: Arc<ShopCache>,
}

impl EconomyContext {
    /// Build an economy context from a base world, defs, and solver options.
    ///
    /// * `base_world` — save world; moved in (unique owner until shared via this context).
    /// * `defs` — game definitions for IO recipes and construction costs.
    /// * `solve_opts` — NLS iteration / warm-start options.
    ///
    /// Builds baseline [`ShopCache`] once via [`ShopCache::from_world`].
    pub fn new(base_world: World, defs: GameDefs, solve_opts: SolveOpts) -> Self {
        let shop_cache = Arc::new(ShopCache::from_world(&base_world, &defs));
        Self {
            base_world,
            defs,
            solve_opts,
            shop_cache,
        }
    }

    /// Empty world + default defs — for timing-only paths that still require economy.
    pub fn empty() -> Self {
        Self::new(World::default(), GameDefs::default(), SolveOpts::default())
    }

    /// Apply this planning branch onto a clone of [`Self::base_world`].
    ///
    /// Applies [`PlanningState::building_level_deltas`] and PM overrides. Used for
    /// placement / construction capacity. Price refresh uses [`Self::shops_for_planning`]
    /// instead (patch baseline shops).
    ///
    /// Types present only via deltas (first-of-type / greenfield) get a
    /// planner-added fully-staffed [`WorldBuilding`] in the delta's `state_id`
    /// so prices and GDP move.
    /// // TODO(buildability): verify free slots / potentials before insert.
    pub(crate) fn apply_planning_to_world(&self, state: &PlanningState) -> World {
        let mut world = self.base_world.clone();
        for ((building, state_id), levels) in &state.building_level_deltas {
            if *levels == 0 {
                continue;
            }
            let mut found = false;
            for row in &mut world.buildings {
                if row.building == *building && row.state == Some(*state_id) {
                    row.add_extra_levels(*levels);
                    found = true;
                }
            }
            if !found {
                if let Some(row) =
                    synthetic_world_building(&self.defs, &world, building, *state_id, *levels)
                {
                    world.buildings.push(row);
                }
            }
        }
        for (building_id, methods) in &state.pm_overrides {
            if let Some(building) = world.buildings.iter_mut().find(|b| b.id == *building_id) {
                *building = building.with_methods(methods.clone());
            }
        }
        world
    }

    /// Materialize shops for this branch: clone baseline, replay deltas as IO patches.
    ///
    /// * `state` — planning node whose `building_level_deltas` / `pm_overrides` apply.
    ///
    /// Throwaway result for one price solve inside apply. Invariant: should match
    /// `ShopCache::from_world(&self.apply_planning_to_world(state))` (parity test).
    pub(crate) fn shops_for_planning(&self, state: &PlanningState) -> ShopCache {
        let mut cache = (*self.shop_cache).clone();
        let n = self.defs.goods_order.len();
        let zeros = vic3_defs::GoodsVec::zeros(n);

        // 1) Extra levels: patch each matching base building (or synthetic greenfield).
        for ((building, state_id), levels) in &state.building_level_deltas {
            if *levels == 0 {
                continue;
            }
            let mut found = false;
            for row in &self.base_world.buildings {
                if row.building == *building && row.state == Some(*state_id) {
                    found = true;
                    let (old_i, old_o) = row.goods_io(&self.defs);
                    let mut bumped = row.clone();
                    bumped.add_extra_levels(*levels);
                    let (new_i, new_o) = bumped.goods_io(&self.defs);
                    cache.patch_building_io(
                        &self.defs,
                        Some(*state_id),
                        &old_i,
                        &old_o,
                        &new_i,
                        &new_o,
                    );
                    cache.set_building_io(row.id, Some(*state_id), new_i, new_o);
                }
            }
            if !found {
                if let Some(syn) = synthetic_world_building(
                    &self.defs,
                    &self.base_world,
                    building,
                    *state_id,
                    *levels,
                ) {
                    let (new_i, new_o) = syn.goods_io(&self.defs);
                    cache.patch_building_io(
                        &self.defs,
                        Some(*state_id),
                        &zeros,
                        &zeros,
                        &new_i,
                        &new_o,
                    );
                    cache.set_building_io(syn.id, Some(*state_id), new_i, new_o);
                }
            }
        }

        // 2) PM overrides: patch from post-level IO with old methods → new methods.
        for (building_id, methods) in &state.pm_overrides {
            let Some(base_row) = self
                .base_world
                .buildings
                .iter()
                .find(|b| b.id == *building_id)
            else {
                continue;
            };
            let mut before = base_row.clone();
            if let Some(sid) = before.state {
                if let Some(levels) = state
                    .building_level_deltas
                    .get(&(before.building.clone(), sid))
                {
                    if *levels > 0 {
                        before.add_extra_levels(*levels);
                    }
                }
            }
            let (old_i, old_o) = before.goods_io(&self.defs);
            let after = before.with_methods(methods.clone());
            let (new_i, new_o) = after.goods_io(&self.defs);
            cache.patch_building_io(&self.defs, before.state, &old_i, &old_o, &new_i, &new_o);
            cache.set_building_io(*building_id, before.state, new_i, new_o);
        }

        cache
    }

    fn owned_state_ids(&self, country: &str) -> BTreeSet<u32> {
        let Some(country_id) = self
            .base_world
            .countries
            .iter()
            .find(|c| c.tag == country)
            .map(|c| c.id)
        else {
            return BTreeSet::new();
        };
        self.base_world
            .states
            .iter()
            .filter_map(|s| (s.country == Some(country_id)).then_some(s.id))
            .collect()
    }

    /// Vic3 placement states for a building type on this branch.
    ///
    /// Prefers states that already have the type (base world, completed deltas,
    /// or in-flight queue). If none, offers every owned state (first-of-type /
    /// greenfield) until `TODO(buildability)` gates on slots.
    fn placement_states_for(
        &self,
        state: &PlanningState,
        world: &World,
        building: &str,
    ) -> Vec<u32> {
        let owned = self.owned_state_ids(&state.country);
        if owned.is_empty() {
            return Vec::new();
        }
        let mut have: BTreeSet<u32> = world
            .buildings
            .iter()
            .filter(|row| row.building == building)
            .filter_map(|row| row.state.filter(|sid| owned.contains(sid)))
            .collect();
        for job in &state.constructions {
            if job.building == building {
                if let Some(sid) = job.state_id.filter(|sid| owned.contains(sid)) {
                    have.insert(sid);
                }
            }
        }
        if have.is_empty() {
            owned.into_iter().collect()
        } else {
            have.into_iter().collect()
        }
    }

    fn levels_added_in_state(state: &PlanningState, building: &str, state_id: u32) -> u32 {
        state
            .building_level_deltas
            .iter()
            .find_map(|((b, sid), n)| (b.as_str() == building && *sid == state_id).then_some(*n))
            .unwrap_or(0)
    }

    /// Goal-relevant `(building_type, state_id)` pairs for `QueueBuildingLevel`.
    ///
    /// Direct types come from defs (IO / GDP / mil PP), not “already in country.”
    /// Each type expands to placement states ([`Self::placement_states_for`]).
    /// Construction Sector is a **meta** type when any direct build is present.
    /// Hire stays on the military simple-subgoal arm.
    ///
    /// No type-level IO dominance prune: cost/benefit axes omit state markets,
    /// slots, and unlocks, so “strictly better type” is not sound.
    ///
    /// // TODO(buildability): before offering a state, require unlock tech and a
    /// // free slot / potential.
    /// //
    /// // TODO(anytime-ub): width may stay large; later PR can compute a greedy
    /// // feasible path as incumbent `U` and prune with `g + h >= U`.
    fn building_candidates(
        &self,
        state: &PlanningState,
        atoms: &[SimpleSubgoal],
        config: SimConfig,
    ) -> Vec<(String, u32)> {
        let cap = config.max_added_levels_per_type;
        let mut types = BTreeSet::new();

        for atom in atoms {
            let SimpleSubgoal::GoodPrice { good, rel, .. } = atom else {
                continue;
            };
            let Some(good_id) = self.defs.index_of(good) else {
                continue;
            };
            for (building_id, building_type) in &self.defs.buildings {
                let (inputs, outputs) = default_building_io_per_level(&self.defs, building_type);
                let produces = outputs[good_id];
                let consumes = inputs[good_id];
                let benefit = match rel {
                    Rel::Le | Rel::Lt => produces,
                    Rel::Ge | Rel::Gt => consumes,
                    Rel::Eq => produces.max(consumes),
                };
                if benefit <= ORDER_EPS || !benefit.is_finite() {
                    continue;
                }
                types.insert(building_id.clone());
            }
        }

        if atoms.iter().any(|atom| {
            matches!(
                atom,
                SimpleSubgoal::Gdp {
                    rel: Rel::Ge | Rel::Gt | Rel::Eq,
                    ..
                }
            )
        }) {
            for (building_id, building_type) in &self.defs.buildings {
                let (_, outputs) = default_building_io_per_level(&self.defs, building_type);
                let benefit = outputs
                    .iter_indexed()
                    .filter(|(_, quantity)| *quantity > ORDER_EPS)
                    .map(|(good, quantity)| {
                        let price = self.defs.good_by_index(good).and_then(|id| {
                            state
                                .price(id)
                                .or_else(|| self.defs.goods.get(id).map(|g| g.base_price))
                        });
                        price.unwrap_or(0.0) * quantity.max(0.0)
                    })
                    .sum::<f64>();
                if benefit <= ORDER_EPS || !benefit.is_finite() {
                    continue;
                }
                types.insert(building_id.clone());
            }
        }

        self.add_military_building_candidates(state, atoms, &mut types, cap);
        if atoms.iter().any(|atom| {
            matches!(
                atom,
                SimpleSubgoal::ArmyPower { .. } | SimpleSubgoal::NavyPower { .. }
            )
        }) {
            self.add_mil_input_producer_candidates(state, &mut types, cap);
        }
        maybe_add_construction_sector_candidate(state, &mut types, cap);

        let world = self.apply_planning_to_world(state);
        let mut out = BTreeSet::new();
        for building in types {
            for state_id in self.placement_states_for(state, &world, &building) {
                if Self::levels_added_in_state(state, &building, state_id) < u32::from(cap) {
                    out.insert((building.clone(), state_id));
                }
            }
        }
        out.into_iter().collect()
    }

    /// Barracks / shipyards / naval admin when open PP atoms need more levels.
    ///
    /// Skipped while that branch is underemployed so hire runs first (see
    /// [`push_military_pp_decisions`]). Types need not already exist in the world.
    fn add_military_building_candidates(
        &self,
        state: &PlanningState,
        atoms: &[SimpleSubgoal],
        candidates: &mut BTreeSet<String>,
        cap: u16,
    ) {
        for atom in atoms {
            match atom {
                SimpleSubgoal::ArmyPower { rel, value } => {
                    if !state.army_buildings_fully_staffed() {
                        continue;
                    }
                    let Some(needed) =
                        power_raise_needed(*rel, *value, state.army_power_projection)
                    else {
                        continue;
                    };
                    if needed <= 0.0 {
                        continue;
                    }
                    let per = UnitCombatStats::army_default()
                        .full_power_projection()
                        .max(1.0);
                    let levels_needed = (needed / per).ceil().max(1.0) as u32;
                    let have = mil_levels(state, is_barracks_building);
                    if have < levels_needed.min(u32::from(cap)) {
                        candidates.insert(BUILDING_BARRACKS.to_string());
                    }
                }
                SimpleSubgoal::NavyPower { rel, value } => {
                    if !state.navy_buildings_fully_staffed() {
                        continue;
                    }
                    let Some(needed) =
                        power_raise_needed(*rel, *value, state.navy_power_projection)
                    else {
                        continue;
                    };
                    if needed <= 0.0 {
                        continue;
                    }
                    let per = UnitCombatStats::navy_default()
                        .full_power_projection()
                        .max(1.0);
                    let levels_needed = (needed / per).ceil().max(1.0) as u32;
                    let ships_needed = levels_needed.min(u32::from(cap));
                    let shipyard = mil_levels(state, is_shipyard_building);
                    let admin = mil_levels(state, is_naval_admin_building);
                    if shipyard < ships_needed {
                        candidates.insert(BUILDING_SHIPYARD.to_string());
                    }
                    if admin < ships_needed {
                        candidates.insert(BUILDING_NAVAL_ADMIN.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn add_mil_input_producer_candidates(
        &self,
        state: &PlanningState,
        candidates: &mut BTreeSet<String>,
        _cap: u16,
    ) {
        let mil_types: Vec<&str> = self
            .defs
            .buildings
            .keys()
            .map(String::as_str)
            .chain([BUILDING_BARRACKS, BUILDING_SHIPYARD, BUILDING_NAVAL_ADMIN])
            .filter(|id| is_military_planning_building(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut expensive_inputs = BTreeSet::new();
        for mil_id in mil_types {
            let Some(building_type) = self.defs.buildings.get(mil_id) else {
                continue;
            };
            let (inputs, _) = default_building_io_per_level(&self.defs, building_type);
            for (good_id, qty) in inputs.iter_indexed() {
                if qty <= ORDER_EPS {
                    continue;
                }
                let Some(good_name) = self.defs.good_by_index(good_id) else {
                    continue;
                };
                let Some(def) = self.defs.goods.get(good_name) else {
                    continue;
                };
                let price = state.price(good_name).unwrap_or(def.base_price);
                if price > def.base_price * MIL_INPUT_PRICE_FACTOR {
                    expensive_inputs.insert(good_id);
                }
            }
        }
        if expensive_inputs.is_empty() {
            return;
        }
        for (building_id, building_type) in &self.defs.buildings {
            let (_, outputs) = default_building_io_per_level(&self.defs, building_type);
            if expensive_inputs
                .iter()
                .any(|good_id| outputs[*good_id] > ORDER_EPS)
            {
                candidates.insert(building_id.clone());
            }
        }
    }

    fn modeled_gdp(&self, state: &PlanningState, outcome: &SolveOutcome) -> f64 {
        let Some(country_id) = self
            .base_world
            .countries
            .iter()
            .find(|country| country.tag == state.country)
            .map(|country| country.id)
        else {
            return 0.0;
        };
        let owned_states: BTreeSet<u32> = self
            .base_world
            .states
            .iter()
            .filter_map(|world_state| {
                (world_state.country == Some(country_id)).then_some(world_state.id)
            })
            .collect();
        outcome
            .building_revenues
            .iter()
            .filter(|building| {
                building
                    .state_id
                    .is_some_and(|state_id| owned_states.contains(&state_id))
            })
            .map(|building| building.revenue.max(0.0))
            .sum()
    }

    /// Goal-relevant alternate production methods, capped for finite branching.
    fn pm_switch_candidates(
        &self,
        state: &PlanningState,
        atoms: &[SimpleSubgoal],
        max_candidates: u16,
        max_overrides: u16,
    ) -> Vec<(u32, Vec<String>)> {
        if state.pm_overrides.len() >= usize::from(max_overrides) || max_candidates == 0 {
            return Vec::new();
        }
        let wants_price = atoms
            .iter()
            .any(|atom| matches!(atom, SimpleSubgoal::GoodPrice { .. }));
        let wants_gdp = atoms.iter().any(|atom| {
            matches!(
                atom,
                SimpleSubgoal::Gdp {
                    rel: Rel::Ge | Rel::Gt | Rel::Eq,
                    ..
                }
            )
        });
        if !wants_price && !wants_gdp {
            return Vec::new();
        }
        let world = self.apply_planning_to_world(state);
        let country_id = world
            .countries
            .iter()
            .find(|country| country.tag == state.country)
            .map(|country| country.id);
        let owned_states: BTreeSet<u32> = world
            .states
            .iter()
            .filter_map(|world_state| (world_state.country == country_id).then_some(world_state.id))
            .collect();
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        'buildings: for building in &world.buildings {
            if building.state.is_some_and(|state_id| {
                !owned_states.is_empty() && !owned_states.contains(&state_id)
            }) {
                continue;
            }
            if state.pm_overrides.contains_key(&building.id) {
                continue;
            }
            let Some(building_type) = self.defs.buildings.get(&building.building) else {
                continue;
            };
            // One Vic3 PM group == one slot. Prefer def group count; fall back to
            // the save's active method list when groups are missing.
            let slot_count = building_type
                .production_method_groups
                .len()
                .max(building.production_methods.len())
                .max(1);
            for slot in 0..slot_count {
                let current = building.production_methods.get(slot).cloned();
                let alternatives = pm_alternatives_for_slot(
                    &self.defs,
                    building_type,
                    building,
                    &world.buildings,
                    slot,
                );
                for candidate in alternatives {
                    if current.as_ref() == Some(&candidate) {
                        continue;
                    }
                    let relevant =
                        pm_affects_open_simple_subgoals(&self.defs, &candidate, atoms, wants_gdp);
                    if !relevant {
                        continue;
                    }
                    let mut methods = if building.production_methods.is_empty() {
                        vec![candidate.clone()]
                    } else {
                        building.production_methods.clone()
                    };
                    // Pad to slot so a missing trailing group can still be set.
                    while methods.len() <= slot {
                        methods.push(String::new());
                    }
                    methods[slot] = candidate;
                    // Drop empty pads if we never filled them from a real current.
                    while methods.last().is_some_and(|m| m.is_empty()) {
                        methods.pop();
                    }
                    if !production_methods_legal_for_building(&self.defs, building_type, &methods) {
                        continue;
                    }
                    if !seen.insert((building.id, methods.clone())) {
                        continue;
                    }
                    out.push((building.id, methods));
                    if out.len() >= usize::from(max_candidates) {
                        break 'buildings;
                    }
                }
            }
        }
        out
    }

    /// True when at least one zero-day PM switch candidate exists for open atoms.
    pub fn has_pm_switch_path(
        &self,
        state: &PlanningState,
        atoms: &[SimpleSubgoal],
        config: SimConfig,
    ) -> bool {
        if !config.allow_pm_changes {
            return false;
        }
        !self
            .pm_switch_candidates(
                state,
                atoms,
                config.max_pm_candidates,
                config.max_pm_overrides,
            )
            .is_empty()
    }
}

/// PMs allowed in `slot` for this building type (one Vic3 PM group per slot).
fn pm_alternatives_for_slot(
    defs: &GameDefs,
    building_type: &BuildingType,
    building: &WorldBuilding,
    peers: &[WorldBuilding],
    slot: usize,
) -> BTreeSet<String> {
    let mut alternatives = BTreeSet::new();
    if let Some(group_id) = building_type.production_method_groups.get(slot) {
        if let Some(group) = defs.production_method_groups.get(group_id) {
            alternatives.extend(group.iter().cloned());
        }
    }
    if !alternatives.is_empty() {
        return alternatives;
    }
    // Defs missing group membership: only peer methods already in this slot index.
    if let Some(current) = building.production_methods.get(slot) {
        alternatives.insert(current.clone());
    }
    for other in peers {
        if other.building == building.building {
            if let Some(pm) = other.production_methods.get(slot) {
                alternatives.insert(pm.clone());
            }
        }
    }
    alternatives
}

/// True when each method belongs to the matching `production_method_groups` slot.
///
/// Rejects cross-group swaps (e.g. the same workshop PM in two slots). When the
/// building type has no groups loaded, only rejects empty method ids.
fn production_methods_legal_for_building(
    defs: &GameDefs,
    building_type: &BuildingType,
    methods: &[String],
) -> bool {
    if methods.is_empty() || methods.iter().any(|m| m.is_empty()) {
        return false;
    }
    let groups = &building_type.production_method_groups;
    if groups.is_empty() {
        return true;
    }
    if methods.len() > groups.len() {
        return false;
    }
    for (slot, pm_id) in methods.iter().enumerate() {
        let Some(group_id) = groups.get(slot) else {
            return false;
        };
        let Some(group) = defs.production_method_groups.get(group_id) else {
            return false;
        };
        if !group.iter().any(|id| id == pm_id) {
            return false;
        }
    }
    true
}

fn pm_affects_open_simple_subgoals(
    defs: &GameDefs,
    pm_id: &str,
    atoms: &[SimpleSubgoal],
    wants_gdp: bool,
) -> bool {
    let Some(pm) = defs.production_methods.get(pm_id) else {
        return wants_gdp;
    };
    if wants_gdp && (!pm.outputs.is_empty() || !pm.inputs.is_empty()) {
        return true;
    }
    for atom in atoms {
        let SimpleSubgoal::GoodPrice { good, rel, .. } = atom else {
            continue;
        };
        let Some(good_id) = defs.index_of(good) else {
            continue;
        };
        let produces = pm
            .outputs
            .iter()
            .any(|(idx, qty)| *idx == good_id && *qty > 0.0);
        let consumes = pm
            .inputs
            .iter()
            .any(|(idx, qty)| *idx == good_id && *qty > 0.0);
        let relevant = match rel {
            Rel::Le | Rel::Lt => produces,
            Rel::Ge | Rel::Gt => consumes,
            Rel::Eq => produces || consumes,
        };
        if relevant {
            return true;
        }
    }
    false
}

/// Tunable durations and branch caps for the compact simulator.
///
/// Defaults are model constants (not Paradox timings). Caps keep A* finite
/// (`max_added_levels_per_type`, `max_pm_*`, `max_tax_steps`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimConfig {
    /// Fixed research duration in the phase-8 model.
    pub research_days: u16,
    /// Fallback duration when a construction job has no known work points.
    pub construction_days: u16,
    /// Days between modeled budget/debt paydays.
    pub payday_days: u16,
    /// Finite search bound for added levels of one building type.
    pub max_added_levels_per_type: u16,
    /// Fixed duration to establish a declared interest (model constant).
    pub interest_days: u16,
    /// Fixed duration to hire barracks toward full employment.
    pub army_training_days: u16,
    /// Fixed duration to crew shipyards / naval administrations.
    pub navy_crew_days: u16,
    /// Fixed duration for one modeled law enactment.
    pub law_days: u16,
    /// Weekly-balance change applied per tax-level step.
    pub tax_balance_per_step: i32,
    /// Absolute tax-level offset cap from the saved baseline.
    pub max_tax_steps: u8,
    /// When false (default), successors never emit [`Action::SwitchPm`].
    ///
    /// PM switches are zero-day and explode the search branching factor; keep
    /// them opt-in via [`crate::plan::PlanOpts::allow_pm_changes`] / CLI.
    pub allow_pm_changes: bool,
    /// Max distinct building PM overrides on one planning branch.
    ///
    /// Ignored when [`Self::allow_pm_changes`] is false.
    pub max_pm_overrides: u16,
    /// Max SwitchPm decision edges emitted in one expansion.
    ///
    /// Ignored when [`Self::allow_pm_changes`] is false.
    pub max_pm_candidates: u16,
    /// National construction capacity with zero Construction Sector levels.
    ///
    /// Positive CS levels add throughput only via defs
    /// `country_construction_add` on the building's required production methods
    /// ([`crate::construction::national_construction_points_per_day`]).
    pub base_construction_capacity: u16,
    /// Optional override for per-job construction points/day from the pool.
    ///
    /// When [`None`] (default), the planner derives the cap from max weekly
    /// construction progress ÷ 7 (game static base + owned tech adds). Set
    /// [`Some`] in tests or as an escape hatch. With government throughput `G`
    /// and cap `A`, up to `floor(G / A)` government jobs progress in parallel.
    pub max_construction_allocation: Option<u16>,
    /// Work points for a new level when defs omit `required_construction`.
    ///
    /// Also used by [`crate::construction::ensure_construction_work_points`] for
    /// save-loaded rows without `remaining`.
    pub default_construction_cost: u16,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            research_days: 365,
            construction_days: 180,
            payday_days: 7,
            max_added_levels_per_type: 10,
            interest_days: 90,
            army_training_days: 90,
            navy_crew_days: 180,
            law_days: 180,
            tax_balance_per_step: 50,
            max_tax_steps: 3,
            allow_pm_changes: false,
            max_pm_overrides: 4,
            max_pm_candidates: 4,
            base_construction_capacity: 1,
            // None → derive from max weekly construction progress / 7.
            max_construction_allocation: None,
            default_construction_cost: 180,
        }
    }
}

impl SimConfig {
    /// Caps used when emitting SwitchPm edges (`(0, _)` when disabled).
    pub const fn pm_branch_caps(self) -> (u16, u16) {
        if self.allow_pm_changes {
            (self.max_pm_candidates, self.max_pm_overrides)
        } else {
            (0, self.max_pm_overrides)
        }
    }
}

/// An event which can advance the simulation clock.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Event {
    /// The technology currently in the queue completes.
    TechCompleted { tech: String },
    /// One level of the queued building type completes.
    /// A building level completed in a placement state.
    BuildingCompleted {
        building: String,
        state_id: Option<u32>,
    },
    /// The queued interest declaration completes.
    InterestDeclared { kind: InterestKind, id: String },
    /// Hire-to-full completes for a military building type.
    HireCompleted { building: String },
    /// The queued law enactment completes.
    LawEnacted { law: String },
    /// One modeled budget tick applies the frozen weekly balance to debt/treasury.
    Payday {},
}

/// Deterministic state transition (decision or wait).
///
/// Decisions cost 0 days when emitted as successors; waits carry `days`
/// (including 0 when a track timer has already elapsed) and advance
/// [`PlanningState::date`] by that amount.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// Put a goal-relevant technology in the empty research queue.
    QueueTech { tech: String },
    /// Put one goal-relevant building level in the construction queue.
    ///
    /// `state_id` is the Vic3 placement state. Sets `remaining` from defs
    /// `required_construction` when present; otherwise
    /// [`SimConfig::default_construction_cost`]. Allowed while other jobs are
    /// in flight until parallel slots from capacity ÷ allocation cap are full.
    /// Construction Sector levels are offered as a capacity lever when other
    /// build candidates already exist for open economy/military atoms.
    QueueBuildingLevel { building: String, state_id: u32 },
    /// Queue a goal-relevant interest declaration.
    QueueInterest { kind: InterestKind, id: String },
    /// Queue hiring a military building type up to full employment.
    QueueHireMilitary { building: String },
    /// Queue enacting a goal-relevant law checkpoint.
    QueueLaw { law: String },
    /// Instantly switch one building to alternate production methods and re-solve.
    SwitchPm {
        building_id: u32,
        methods: Vec<String>,
    },
    /// Adjust tax level by `delta` and apply `balance_delta_bits` (`f64::to_bits`)
    /// to the frozen weekly-balance sample.
    AdjustTax { delta: i8, balance_delta_bits: u64 },
    /// Advance directly to an event already in flight.
    WaitForEvent { event: Event, days: u16 },
}

/// One edge emitted by [`successors`] or [`successors_for_simple_subgoals`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Successor {
    pub action: Action,
    /// Edge cost in days. Decision edges have cost zero.
    pub days: u16,
    pub state: PlanningState,
}

/// Generate successors relevant to the currently unsatisfied atoms of `goal`.
///
/// When `economy.defs` carries technologies, open research gaps expand to
/// missing ancestors and only prereq-satisfied techs are queued.
pub fn successors(
    state: &PlanningState,
    goal: &Goal,
    config: SimConfig,
    economy: &EconomyContext,
) -> Vec<Successor> {
    let open_simple_subgoals = if economy.defs.technologies.is_empty() {
        gaps(goal, state)
    } else {
        crate::goals::gaps_with_defs(goal, state, &economy.defs)
    };
    successors_for_simple_subgoals(state, &open_simple_subgoals, config, economy)
}

/// Alias for [`successors`] (historical name from the optional-economy era).
pub fn successors_with_economy(
    state: &PlanningState,
    goal: &Goal,
    config: SimConfig,
    economy: &EconomyContext,
) -> Vec<Successor> {
    successors(state, goal, config, economy)
}

/// Generate successors from an already-computed list of open goal atoms.
///
/// Decisions are gated per track (`research_busy`, `construction_busy`, …) so
/// research and construction may run in parallel. Instant `SwitchPm` /
/// `AdjustTax` decisions are not blocked by other tracks. At most one wait
/// edge is appended — the earliest completion among goal-relevant in-flight
/// tracks, or a payday tick when a solvency-related simple subgoal can move closer.
/// Idle states without an open solvency path emit no wait (I6).
pub fn successors_for_simple_subgoals(
    state: &PlanningState,
    open_simple_subgoals: &[SimpleSubgoal],
    config: SimConfig,
    economy: &EconomyContext,
) -> Vec<Successor> {
    successors_for_simple_subgoals_with_economy(state, open_simple_subgoals, config, economy)
}

fn interest_queued(kind: InterestKind, id: &str) -> QueuedInterest {
    match kind {
        InterestKind::State => QueuedInterest::State(id.to_string()),
        InterestKind::Region => QueuedInterest::Region(id.to_string()),
    }
}

/// Target army power that would satisfy an open comparison by raising projection.
///
/// Unknown current projection (`None`), lower-bound atoms that already hold,
/// equality from above, or upper-bound atoms that cannot be closed by increasing
/// power, yield `None`.
pub fn power_raise_needed(rel: Rel, value: f64, current: Option<f64>) -> Option<f64> {
    let current = current?;
    if !value.is_finite() {
        return None;
    }
    match rel {
        Rel::Ge => (!rel.holds(current, value)).then_some(value - current),
        Rel::Eq => (current < value).then_some(value - current),
        Rel::Gt => (!rel.holds(current, value)).then_some(value + 1.0 - current),
        Rel::Le | Rel::Lt => None,
    }
}

fn push_decision(
    result: &mut Vec<Successor>,
    state: &PlanningState,
    action: Action,
    economy: &EconomyContext,
    config: SimConfig,
) {
    if let Some(next) = apply_action(state, &action, economy, config) {
        result.push(Successor {
            action,
            days: 0,
            state: next,
        });
    }
}

fn push_wait(
    result: &mut Vec<Successor>,
    state: &PlanningState,
    event: Event,
    days: u16,
    economy: &EconomyContext,
    config: SimConfig,
) {
    // `days == 0` is allowed when the track timer has already elapsed.
    let action = Action::WaitForEvent { event, days };
    if let Some(next) = apply_action(state, &action, economy, config) {
        result.push(Successor {
            action,
            days,
            state: next,
        });
    }
}

fn successors_for_simple_subgoals_with_economy(
    state: &PlanningState,
    open_simple_subgoals: &[SimpleSubgoal],
    config: SimConfig,
    economy: &EconomyContext,
) -> Vec<Successor> {
    let mut result = Vec::new();
    let mut seen_techs = BTreeSet::new();
    let mut seen_laws = BTreeSet::new();
    let mut seen_interest = BTreeSet::new();
    let mut seen_mil_hires = BTreeSet::new();
    let mut seen_tax_deltas = BTreeSet::new();

    for atom in open_simple_subgoals {
        match atom {
            SimpleSubgoal::HasTech(tech) => {
                if state.research_busy()
                    || state.has_tech(tech)
                    || !seen_techs.insert(tech.clone())
                    || !crate::tech::tech_prereqs_satisfied(tech, state, &economy.defs)
                {
                    continue;
                }
                push_decision(
                    &mut result,
                    state,
                    Action::QueueTech { tech: tech.clone() },
                    economy,
                    config,
                );
            }
            SimpleSubgoal::HasLaw(law) => {
                if state.law_busy()
                    || state.has_law(law)
                    || !seen_laws.insert(crate::world::law_key(law))
                {
                    continue;
                }
                push_decision(
                    &mut result,
                    state,
                    Action::QueueLaw { law: law.clone() },
                    economy,
                    config,
                );
            }
            SimpleSubgoal::InterestIn { kind, id } => {
                let key = (*kind, id.clone());
                if state.interest_busy() || atom.eval(state) || !seen_interest.insert(key) {
                    continue;
                }
                push_decision(
                    &mut result,
                    state,
                    Action::QueueInterest {
                        kind: *kind,
                        id: id.clone(),
                    },
                    economy,
                    config,
                );
            }
            SimpleSubgoal::ArmyPower { rel, value } => {
                push_military_pp_decisions(
                    &mut MilitaryPpDecisionArgs {
                        result: &mut result,
                        state,
                        economy,
                        config,
                        seen_hires: &mut seen_mil_hires,
                    },
                    MilitaryBranch::Army,
                    *rel,
                    *value,
                );
            }
            SimpleSubgoal::NavyPower { rel, value } => {
                push_military_pp_decisions(
                    &mut MilitaryPpDecisionArgs {
                        result: &mut result,
                        state,
                        economy,
                        config,
                        seen_hires: &mut seen_mil_hires,
                    },
                    MilitaryBranch::Navy,
                    *rel,
                    *value,
                );
            }
            SimpleSubgoal::WeeklyBalance { .. } => {
                for delta in tax_deltas_toward(state, atom, config) {
                    if !seen_tax_deltas.insert(delta) {
                        continue;
                    }
                    let balance_delta = f64::from(config.tax_balance_per_step) * f64::from(delta);
                    push_decision(
                        &mut result,
                        state,
                        Action::AdjustTax {
                            delta,
                            balance_delta_bits: balance_delta.to_bits(),
                        },
                        economy,
                        config,
                    );
                }
            }
            _ => {}
        }
    }

    for (building, state_id) in economy.building_candidates(state, open_simple_subgoals, config) {
        push_decision(
            &mut result,
            state,
            Action::QueueBuildingLevel { building, state_id },
            economy,
            config,
        );
    }
    let (max_pm_candidates, max_pm_overrides) = config.pm_branch_caps();
    for (building_id, methods) in economy.pm_switch_candidates(
        state,
        open_simple_subgoals,
        max_pm_candidates,
        max_pm_overrides,
    ) {
        push_decision(
            &mut result,
            state,
            Action::SwitchPm {
                building_id,
                methods,
            },
            economy,
            config,
        );
    }

    // Earliest wait among independent tracks.
    //
    // Construction already waits for whatever is in flight. Research / law /
    // interest must do the same when open atoms still need that track: an
    // unrelated save queue (e.g. researching `canneries` while the goal needs
    // `nitroglycerin`) would otherwise block `QueueTech` and emit no wait →
    // false [`crate::plan::PlanError::Unreachable`].
    let mut wait_candidates: Vec<(u16, Event)> = Vec::new();
    if let Some(tech) = state.queued_tech.as_ref().filter(|_| {
        open_simple_subgoals
            .iter()
            .any(|atom| matches!(atom, SimpleSubgoal::HasTech(id) if !state.has_tech(id)))
    }) {
        let days = state
            .tech_days_left
            .unwrap_or_else(|| research_days_for_tech(tech, config, economy));
        wait_candidates.push((days, Event::TechCompleted { tech: tech.clone() }));
    }
    if state.queued_building.is_some() {
        if let Some((days, building, state_id)) = construction_wait_target(state, config) {
            wait_candidates.push((days, Event::BuildingCompleted { building, state_id }));
        }
    }
    if let Some(queued) = state.queued_interest.as_ref().filter(|_| {
        open_simple_subgoals.iter().any(|atom| match atom {
            SimpleSubgoal::InterestIn { kind, id } => match kind {
                InterestKind::State => !state.has_interest_state(id),
                InterestKind::Region => !state.has_interest_region(id),
            },
            _ => false,
        })
    }) {
        let (kind, id) = match queued {
            QueuedInterest::State(id) => (InterestKind::State, id.clone()),
            QueuedInterest::Region(id) => (InterestKind::Region, id.clone()),
        };
        let days = state.interest_days_left.unwrap_or(config.interest_days);
        wait_candidates.push((days, Event::InterestDeclared { kind, id }));
    }
    if let Some(building) = state.queued_hire.as_ref().filter(|queued| {
        open_simple_subgoals.iter().any(|atom| {
            matches!(
                atom,
                SimpleSubgoal::ArmyPower { .. } | SimpleSubgoal::NavyPower { .. }
            ) && !atom.eval(state)
        }) && state
            .mil_buildings
            .iter()
            .any(|row| row.building == **queued && !row.is_fully_staffed())
    }) {
        let default_days = if is_barracks_building(building) {
            config.army_training_days
        } else {
            config.navy_crew_days
        };
        let days = state.hire_days_left.unwrap_or(default_days);
        wait_candidates.push((
            days,
            Event::HireCompleted {
                building: building.clone(),
            },
        ));
    }
    if let Some(law) = state.queued_law.as_ref().filter(|_| {
        open_simple_subgoals
            .iter()
            .any(|atom| matches!(atom, SimpleSubgoal::HasLaw(id) if !state.has_law(id)))
    }) {
        let days = state.law_days_left.unwrap_or(config.law_days);
        wait_candidates.push((days, Event::LawEnacted { law: law.clone() }));
    }
    if payday_can_help(state, open_simple_subgoals) {
        wait_candidates.push((config.payday_days, Event::Payday {}));
    }

    if let Some((days, event)) = wait_candidates.into_iter().min_by_key(|(days, _)| *days) {
        push_wait(&mut result, state, event, days, economy, config);
    }

    result
}

/// Research duration for a queued tech: defs cost at [`crate::tracks::CONSTANT_RATE`]
/// when present, otherwise [`SimConfig::research_days`].
fn research_days_for_tech(tech: &str, config: SimConfig, economy: &EconomyContext) -> u16 {
    if let Some(cost) = crate::tech::tech_research_cost(tech, &economy.defs) {
        if let Some(days) = crate::tracks::days_for_work(cost, crate::tracks::CONSTANT_RATE) {
            return u16::try_from(days).unwrap_or(u16::MAX).max(1);
        }
    }
    config.research_days.max(1)
}

fn tax_deltas_toward(state: &PlanningState, atom: &SimpleSubgoal, config: SimConfig) -> Vec<i8> {
    let SimpleSubgoal::WeeklyBalance { rel, value } = atom else {
        return Vec::new();
    };
    let Some(balance) = state.weekly_balance.filter(|v| v.is_finite()) else {
        return Vec::new();
    };
    if rel.holds(balance, *value) {
        return Vec::new();
    }
    let step = f64::from(config.tax_balance_per_step);
    if step <= 0.0 {
        return Vec::new();
    }
    let max_steps = i8::try_from(config.max_tax_steps).unwrap_or(3);
    let mut out = Vec::new();
    for delta in [-1_i8, 1_i8] {
        let next_level = state.tax_level.saturating_add(delta);
        if next_level.abs() > max_steps {
            continue;
        }
        let next_balance = balance + step * f64::from(delta);
        let before = (balance - *value).abs();
        let after = (next_balance - *value).abs();
        let improves = match rel {
            Rel::Ge | Rel::Gt => next_balance > balance && !rel.holds(balance, *value),
            Rel::Le | Rel::Lt => next_balance < balance && !rel.holds(balance, *value),
            Rel::Eq => after < before,
        };
        if improves {
            out.push(delta);
        }
    }
    out
}

fn is_solvency_simple_subgoal(atom: &SimpleSubgoal) -> bool {
    matches!(
        atom,
        SimpleSubgoal::Solvent
            | SimpleSubgoal::CreditHeadroom { .. }
            | SimpleSubgoal::DebtPrincipal { .. }
    )
}

/// Apply one frozen weekly-balance sample to treasury and debt principal, then
/// refresh `credit_headroom` / `solvent`. This is not Paradox's full budget.
fn apply_payday_effects(state: &mut PlanningState) {
    let Some(balance) = state.weekly_balance.filter(|value| value.is_finite()) else {
        return;
    };
    let mut treasury = state.treasury;
    let mut principal = state.debt_principal.unwrap_or(0.0);

    if balance >= 0.0 {
        let mut remaining = balance;
        if principal > 0.0 {
            let pay = remaining.min(principal);
            principal -= pay;
            remaining -= pay;
        }
        treasury += remaining;
    } else {
        let need = -balance;
        if treasury >= need {
            treasury -= need;
        } else {
            principal += need - treasury;
            treasury = 0.0;
        }
    }

    state.treasury = treasury;
    if state.debt_principal.is_some() || state.credit_limit.is_some() || principal > 0.0 {
        state.debt_principal = Some(principal);
    }
    state.credit_headroom = match (state.debt_principal, state.credit_limit) {
        (Some(principal), Some(credit)) if principal.is_finite() && credit.is_finite() => {
            Some(credit - principal)
        }
        _ => None,
    };
    state.solvent = state
        .credit_headroom
        .map(|headroom| headroom > 0.0)
        .unwrap_or(false);
}

fn fiscal_slack(atom: &SimpleSubgoal, state: &PlanningState) -> Option<f64> {
    match atom {
        SimpleSubgoal::Solvent => Some(if state.solvent { 0.0 } else { 1.0 }),
        SimpleSubgoal::CreditHeadroom { rel, value } => {
            let headroom = state.credit_headroom?;
            if rel.holds(headroom, *value) {
                Some(0.0)
            } else {
                Some((headroom - *value).abs())
            }
        }
        SimpleSubgoal::DebtPrincipal { rel, value } => {
            let principal = state.debt_principal?;
            if rel.holds(principal, *value) {
                Some(0.0)
            } else {
                Some((principal - *value).abs())
            }
        }
        _ => None,
    }
}

/// Emit payday only when a solvency-related open atom can move closer after one
/// tick. Prevents idle wait loops when the frozen balance cannot help.
fn payday_can_help(state: &PlanningState, open_simple_subgoals: &[SimpleSubgoal]) -> bool {
    if !open_simple_subgoals.iter().any(is_solvency_simple_subgoal) {
        return false;
    }
    let mut next = state.clone();
    apply_payday_effects(&mut next);
    if next == *state {
        return false;
    }
    open_simple_subgoals.iter().any(|atom| {
        let Some(before) = fiscal_slack(atom, state) else {
            return false;
        };
        let Some(after) = fiscal_slack(atom, &next) else {
            return false;
        };
        after < before
    })
}

/// Apply an action if its preconditions hold.
///
/// Timing comes from `config` (queue durations) and the action's wait `days`.
/// Applying the same `(state, action, economy, config)` always yields the same
/// result (I8). `economy` is required for building / PM / price refresh paths.
pub fn apply_action(
    state: &PlanningState,
    action: &Action,
    economy: &EconomyContext,
    config: SimConfig,
) -> Option<PlanningState> {
    let mut next = state.clone();
    match action {
        Action::QueueTech { tech } => {
            if tech.is_empty()
                || next.research_busy()
                || next.has_tech(tech)
                || !crate::tech::tech_prereqs_satisfied(tech, &next, &economy.defs)
            {
                return None;
            }
            next.queued_tech = Some(tech.clone());
            next.tech_days_left = Some(research_days_for_tech(tech, config, economy));
        }
        Action::QueueBuildingLevel { building, state_id } => {
            if building.is_empty() || construction_queue_full(&next, config) {
                return None;
            }
            if !economy.owned_state_ids(&next.country).contains(state_id) {
                return None;
            }
            let remaining = construction_work_points_for_enqueue(building, economy, config);
            next.push_construction(building.clone(), *state_id, remaining);
        }
        Action::QueueInterest { kind, id } => {
            if id.is_empty() || next.interest_busy() {
                return None;
            }
            let already = match kind {
                InterestKind::State => next.has_interest_state(id),
                InterestKind::Region => next.has_interest_region(id),
            };
            if already {
                return None;
            }
            next.queued_interest = Some(interest_queued(*kind, id));
            next.interest_days_left = Some(config.interest_days);
        }
        Action::QueueHireMilitary { building } => {
            if building.is_empty() || next.hire_busy() {
                return None;
            }
            let row = next
                .mil_buildings
                .iter()
                .find(|row| row.building == *building)?;
            if row.is_fully_staffed() {
                return None;
            }
            next.queued_hire = Some(building.clone());
            next.hire_days_left = Some(if is_barracks_building(building) {
                config.army_training_days
            } else {
                config.navy_crew_days
            });
        }
        Action::QueueLaw { law } => {
            if law.is_empty() || next.law_busy() || next.has_law(law) {
                return None;
            }
            next.queued_law = Some(law.clone());
            next.law_days_left = Some(config.law_days);
        }
        Action::SwitchPm {
            building_id,
            methods,
        } => {
            if methods.is_empty() {
                return None;
            }
            if next.pm_overrides.get(building_id) == Some(methods) {
                return None;
            }
            // Must exist on the projected branch (base or after earlier deltas).
            let world = economy.apply_planning_to_world(&next);
            if !world.buildings.iter().any(|b| b.id == *building_id) {
                return None;
            }
            next.pm_overrides.insert(*building_id, methods.clone());
            refresh_prices(&mut next, economy);
        }
        Action::AdjustTax {
            delta,
            balance_delta_bits,
        } => {
            if *delta == 0 {
                return None;
            }
            let balance_delta = f64::from_bits(*balance_delta_bits);
            if !balance_delta.is_finite() {
                return None;
            }
            let balance = next.weekly_balance.filter(|v| v.is_finite())?;
            next.tax_level = next.tax_level.saturating_add(*delta);
            next.weekly_balance = Some(balance + balance_delta);
        }
        Action::WaitForEvent { event, days } => {
            apply_wait_for_event(&mut next, event, *days, economy, config)?;
        }
    }
    Some(next)
}

/// Alias for [`apply_action`] (historical name from the optional-economy era).
pub fn apply_action_with_economy(
    state: &PlanningState,
    action: &Action,
    economy: &EconomyContext,
    config: SimConfig,
) -> Option<PlanningState> {
    apply_action(state, action, economy, config)
}

/// Failure modes for [`speculative_completed_state`] — never soft-fail with `None`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum SpeculativeCompleteError {
    #[error("action could not be applied for speculative complete")]
    ApplyRejected,
    #[error("construction wait days unavailable for speculative complete")]
    WaitUnavailable,
}

/// Build a planning world where `action`'s economics have landed, without mutating
/// the caller's enqueue-only search state.
///
/// # What “completed” means
///
/// - [`Action::QueueBuildingLevel`]: apply the enqueue, then advance through
///   [`Event::BuildingCompleted`] for that job alone using
///   [`construction_wait_days`]. Level deltas, construction-point sync, and
///   price/GDP refresh match a finished build. Shorter intervening waits on
///   other tracks are not modeled.
/// - Other actions: identical to [`apply_action_with_economy`] (already
///   instant / no separate completion phase in this sim).
///
/// `economy` is required: queue/complete paths need defs and price refresh.
#[allow(dead_code)] // score/emit callers land in a later PR
pub(crate) fn speculative_completed_state(
    state: &PlanningState,
    action: &Action,
    economy: &EconomyContext,
    config: SimConfig,
) -> Result<PlanningState, SpeculativeCompleteError> {
    match action {
        Action::QueueBuildingLevel { building, state_id } => {
            let queued = apply_action(state, action, economy, config)
                .ok_or(SpeculativeCompleteError::ApplyRejected)?;
            let wait_days = construction_wait_days(&queued, config)
                .ok_or(SpeculativeCompleteError::WaitUnavailable)?;
            apply_action(
                &queued,
                &Action::WaitForEvent {
                    event: Event::BuildingCompleted {
                        building: building.clone(),
                        state_id: Some(*state_id),
                    },
                    days: wait_days,
                },
                economy,
                config,
            )
            .ok_or(SpeculativeCompleteError::ApplyRejected)
        }
        _ => apply_action(state, action, economy, config)
            .ok_or(SpeculativeCompleteError::ApplyRejected),
    }
}

/// Seed missing track timers from `config` so parallel ticks advance save-loaded queues.
///
/// Research / interest / hire / law get their fixed-day counters when absent.
/// Construction rows without `remaining` are filled via
/// [`ensure_construction_work_points`].
fn ensure_track_timers(state: &mut PlanningState, config: SimConfig) {
    if state.queued_tech.is_some() && state.tech_days_left.is_none() {
        state.tech_days_left = Some(config.research_days);
    }
    if state.queued_interest.is_some() && state.interest_days_left.is_none() {
        state.interest_days_left = Some(config.interest_days);
    }
    if state.queued_hire.is_some() && state.hire_days_left.is_none() {
        let default_days = state
            .queued_hire
            .as_deref()
            .map(|building| {
                if is_barracks_building(building) {
                    config.army_training_days
                } else {
                    config.navy_crew_days
                }
            })
            .unwrap_or(config.army_training_days);
        state.hire_days_left = Some(default_days);
    }
    if state.queued_law.is_some() && state.law_days_left.is_none() {
        state.law_days_left = Some(config.law_days);
    }
    ensure_construction_work_points(state, config);
}

fn apply_wait_for_event(
    next: &mut PlanningState,
    event: &Event,
    days: u16,
    economy: &EconomyContext,
    config: SimConfig,
) -> Option<()> {
    ensure_track_timers(next, config);
    match event {
        Event::TechCompleted { tech } => {
            if next.queued_tech.as_deref() != Some(tech.as_str()) {
                return None;
            }
            if days == 0 && next.tech_days_left != Some(0) {
                return None;
            }
            next.tick_parallel_tracks(days, &construction_points_per_day_per_job(next, config));
            next.date = next.date.add_days(i32::from(days));
            next.queued_tech = None;
            next.tech_days_left = None;
            next.techs.insert(tech.clone());
        }
        Event::BuildingCompleted { building, state_id } => {
            if !next.constructions.iter().any(|row| {
                row.building == *building
                    && state_id
                        .map(|want| row.state_id == Some(want) || row.state_id.is_none())
                        .unwrap_or(true)
            }) {
                return None;
            }
            if days == 0 && !construction_work_complete(next, building, *state_id) {
                return None;
            }
            next.tick_parallel_tracks(days, &construction_points_per_day_per_job(next, config));
            next.date = next.date.add_days(i32::from(days));
            next.complete_construction(building, *state_id);
            if let Some(sid) = *state_id {
                *next
                    .building_level_deltas
                    .entry((building.clone(), sid))
                    .or_default() += 1;
            } else if let Some(sid) = economy
                .owned_state_ids(&next.country)
                .iter()
                .next()
                .copied()
            {
                // Save rows without placement: attribute to an owned state so
                // apply_planning_to_world still moves levels.
                *next
                    .building_level_deltas
                    .entry((building.clone(), sid))
                    .or_default() += 1;
            }
            if is_military_planning_building(building) {
                next.push_mil_building_level(building);
            }
            if building == BUILDING_CONSTRUCTION_SECTOR {
                sync_construction_points_per_day(next, economy, config);
            }
            refresh_prices(next, economy);
        }
        Event::InterestDeclared { kind, id } => {
            let expected = interest_queued(*kind, id);
            if next.queued_interest.as_ref() != Some(&expected) {
                return None;
            }
            if days == 0 && next.interest_days_left != Some(0) {
                return None;
            }
            next.tick_parallel_tracks(days, &construction_points_per_day_per_job(next, config));
            next.date = next.date.add_days(i32::from(days));
            next.queued_interest = None;
            next.interest_days_left = None;
            match kind {
                InterestKind::State => {
                    next.interest_states.insert(id.clone());
                }
                InterestKind::Region => {
                    next.interest_regions.insert(id.clone());
                }
            }
        }
        Event::HireCompleted { building } => {
            if next.queued_hire.as_deref() != Some(building.as_str()) {
                return None;
            }
            if days == 0 && next.hire_days_left != Some(0) {
                return None;
            }
            next.tick_parallel_tracks(days, &construction_points_per_day_per_job(next, config));
            next.date = next.date.add_days(i32::from(days));
            next.queued_hire = None;
            next.hire_days_left = None;
            next.complete_mil_hire(building);
        }
        Event::LawEnacted { law } => {
            if next.queued_law.as_deref() != Some(law.as_str()) {
                return None;
            }
            if days == 0 && next.law_days_left != Some(0) {
                return None;
            }
            next.tick_parallel_tracks(days, &construction_points_per_day_per_job(next, config));
            next.date = next.date.add_days(i32::from(days));
            next.queued_law = None;
            next.law_days_left = None;
            next.laws.insert(law.clone());
        }
        Event::Payday {} => {
            if days == 0 {
                return None;
            }
            next.tick_parallel_tracks(days, &construction_points_per_day_per_job(next, config));
            next.date = next.date.add_days(i32::from(days));
            apply_payday_effects(next);
        }
    }
    Some(())
}

/// Write solved prices/GDP onto `state` after an economy-relevant transition.
///
/// Search calls apply only; this runs inside apply. Bookkeeping: baseline shops
/// → patch from deltas → NLS → node fields.
fn refresh_prices(state: &mut PlanningState, economy: &EconomyContext) {
    let cache = economy.shops_for_planning(state);
    let outcome = equilibrate_cached(&cache, &economy.defs, economy.solve_opts.clone());
    state.gdp = economy.modeled_gdp(state, &outcome);
    state.good_prices = outcome
        .goods
        .into_iter()
        .map(|good| (good.name, good.price))
        .collect();
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construction::{construction_wait_days, max_parallel_construction_jobs};
    use crate::goals::{compile, evaluate};
    use crate::world::{PlanningParts, Vic3Date};
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use vic3_defs::{Good, GoodId, GoodsVec};
    use vic3_prices::{solve, WorldBuilding, WorldCountry, WorldState};

    #[test]
    fn version_is_semver() {
        assert!(!super::version().is_empty());
    }

    fn state_at(day_offset: i32) -> PlanningState {
        PlanningState::from_parts(PlanningParts {
            date: Vic3Date::from_ymdh(1836, 1, 1, 0).add_days(day_offset),
            country: "GER".into(),
            ..PlanningParts::default()
        })
    }

    #[test]
    fn queue_interest_then_wait_reaches_interest_in() {
        let goal = compile("interest_in(state=alsace)").unwrap();
        let start = state_at(0);
        let config = SimConfig {
            interest_days: 45,
            ..SimConfig::default()
        };

        let decisions = successors(&start, &goal, config, &EconomyContext::empty());
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].days, 0);
        assert!(matches!(
            decisions[0].action,
            Action::QueueInterest {
                kind: InterestKind::State,
                ref id
            } if id == "alsace"
        ));

        let waits = successors(&decisions[0].state, &goal, config, &EconomyContext::empty());
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 45);
        assert!(waits[0].state.has_interest_state("alsace"));
        assert!(evaluate(&goal, &waits[0].state));
    }

    #[test]
    fn queue_hire_navy_buildings_raises_navy_pp() {
        use crate::military::{
            ModeledMilBuilding, UnitCombatStats, BUILDING_NAVAL_ADMIN, BUILDING_SHIPYARD,
        };
        let per = UnitCombatStats::navy_default().full_power_projection();
        let levels = (100.0 / per).ceil();
        let goal = compile("navy_power_projection >= 100").unwrap();
        let start = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            navy_power_projection: Some(0.0),
            navy_pp_baseline: Some(0.0),
            mil_buildings: vec![
                ModeledMilBuilding {
                    building: BUILDING_SHIPYARD.into(),
                    levels,
                    staffing: 0.0,
                },
                ModeledMilBuilding {
                    building: BUILDING_NAVAL_ADMIN.into(),
                    levels,
                    staffing: 0.0,
                },
            ],
            ..PlanningParts::default()
        });
        let config = SimConfig {
            navy_crew_days: 40,
            ..SimConfig::default()
        };
        // Hire shipyard then admin (order may vary); drain until goal holds.
        let mut state = start;
        for _ in 0..6 {
            if evaluate(&goal, &state) {
                break;
            }
            let edges = successors(&state, &goal, config, &EconomyContext::empty());
            assert!(!edges.is_empty(), "expected hire/wait edges at {state:?}");
            state = edges[0].state.clone();
        }
        assert!(evaluate(&goal, &state));
        assert!(state.navy_power_projection.is_some_and(|p| p >= 100.0));
    }

    #[test]
    fn queue_hire_barracks_then_wait_reaches_army_power() {
        use crate::military::{ModeledMilBuilding, UnitCombatStats, BUILDING_BARRACKS};
        let per = UnitCombatStats::army_default().full_power_projection();
        let levels = (100.0 / per).ceil();
        let goal = compile("army_power_projection >= 100").unwrap();
        let start = PlanningState::from_parts(PlanningParts {
            date: Vic3Date::from_ymdh(1836, 1, 1, 0),
            country: "GER".into(),
            army_power_projection: Some(0.0),
            army_pp_baseline: Some(0.0),
            mil_buildings: vec![ModeledMilBuilding {
                building: BUILDING_BARRACKS.into(),
                levels,
                staffing: 0.0,
            }],
            ..PlanningParts::default()
        });
        let config = SimConfig {
            army_training_days: 60,
            ..SimConfig::default()
        };

        let decisions = successors(&start, &goal, config, &EconomyContext::empty());
        assert_eq!(decisions.len(), 1);
        assert!(matches!(
            decisions[0].action,
            Action::QueueHireMilitary { ref building } if building == BUILDING_BARRACKS
        ));

        let waits = successors(&decisions[0].state, &goal, config, &EconomyContext::empty());
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 60);
        assert!(waits[0]
            .state
            .army_power_projection
            .is_some_and(|p| p >= 100.0));
        assert!(evaluate(&goal, &waits[0].state));
    }

    #[test]
    fn unknown_army_power_does_not_queue_expand() {
        let goal = compile("army_power_projection >= 100").unwrap();
        let start = state_at(0);
        assert_eq!(start.army_power_projection, None);
        let decisions = successors(
            &start,
            &goal,
            SimConfig::default(),
            &EconomyContext::empty(),
        );
        assert!(
            decisions.iter().all(|s| {
                !matches!(
                    s.action,
                    Action::QueueHireMilitary { .. } | Action::QueueBuildingLevel { .. }
                )
            }),
            "unknown PP must not look like actionable zero: {decisions:?}"
        );
    }

    #[test]
    fn queue_tech_then_wait_reaches_has_tech() {
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let start = state_at(0);

        let decisions = successors(
            &start,
            &goal,
            SimConfig {
                research_days: 100,
                ..SimConfig::default()
            },
            &EconomyContext::empty(),
        );
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].days, 0);
        assert!(matches!(
            decisions[0].action,
            Action::QueueTech { ref tech } if tech == "nitroglycerin"
        ));

        let waits = successors(
            &decisions[0].state,
            &goal,
            SimConfig {
                research_days: 100,
                ..SimConfig::default()
            },
            &EconomyContext::empty(),
        );
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 100);
        assert_eq!(start.date.days_until(&waits[0].state.date), 100);
        assert!(waits[0].state.queued_tech.is_none());
        assert!(evaluate(&goal, &waits[0].state));
    }

    #[test]
    fn unrelated_inflight_research_can_wait_to_free_track() {
        // Save often has research_queue on a tech that is not the goal leaf.
        // Waiting must still be offered so A* can clear the track then queue
        // goal techs (otherwise false Unreachable).
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let start = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_tech: Some("canneries".into()),
            tech_days_left: Some(40),
            ..PlanningParts::default()
        });
        let edges = successors(
            &start,
            &goal,
            SimConfig {
                research_days: 100,
                ..SimConfig::default()
            },
            &EconomyContext::empty(),
        );
        assert!(
            edges.iter().any(|edge| {
                matches!(
                    &edge.action,
                    Action::WaitForEvent {
                        event: Event::TechCompleted { tech },
                        days: 40,
                    } if tech == "canneries"
                )
            }),
            "expected wait for in-flight canneries among {edges:?}"
        );
        assert!(
            edges
                .iter()
                .all(|edge| { !matches!(&edge.action, Action::QueueTech { .. }) }),
            "must not enqueue while research busy: {edges:?}"
        );
    }

    #[test]
    fn with_tech_defs_only_eligible_prereq_leaf_is_queued() {
        use vic3_defs::Technology;

        let mut technologies = BTreeMap::new();
        technologies.insert(
            "manufacturies".into(),
            Technology {
                id: "manufacturies".into(),
                cost: Some(50.0),
                prerequisites: vec![],
            },
        );
        technologies.insert(
            "shaft_mining".into(),
            Technology {
                id: "shaft_mining".into(),
                cost: Some(75.0),
                prerequisites: vec!["manufacturies".into()],
            },
        );
        technologies.insert(
            "nitroglycerin".into(),
            Technology {
                id: "nitroglycerin".into(),
                cost: Some(100.0),
                prerequisites: vec!["shaft_mining".into()],
            },
        );
        let defs = GameDefs {
            technologies,
            ..GameDefs::default()
        };
        let economy = EconomyContext::new(World::default(), defs, SolveOpts::default());
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let start = state_at(0);
        let decisions = successors_with_economy(&start, &goal, SimConfig::default(), &economy);
        assert_eq!(decisions.len(), 1);
        assert!(matches!(
            decisions[0].action,
            Action::QueueTech { ref tech } if tech == "manufacturies"
        ));
        assert_eq!(decisions[0].state.tech_days_left, Some(50));
    }

    #[test]
    fn building_level_then_wait_reaches_good_price() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodId::from_usize(0);
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_logging".into()],
                required_construction: Some(30.0),
            },
        );
        defs.production_method_groups
            .insert("pmg_logging".into(), vec!["pm_sawmills".into()]);
        defs.production_methods.insert(
            "pm_sawmills".into(),
            ProductionMethod {
                id: "pm_sawmills".into(),
                outputs: vec![(wood, 10.0)],
                ..Default::default()
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_sawmills".into()],
                saved_inputs: Vec::new(),
                saved_outputs: vec![(wood, 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let solve_opts = SolveOpts::default();
        let baseline = solve(&world, &defs, solve_opts.clone());
        let bumped = solve(
            &world.with_extra_levels("building_logging_camp", 1),
            &defs,
            solve_opts.clone(),
        );
        let initial_price = baseline.goods[0].price;
        let next_price = bumped.goods[0].price;
        assert!(next_price < initial_price);
        let initial_gdp = baseline.buildings[0].revenue;
        let next_gdp = bumped.buildings[0].revenue;
        assert!(next_gdp > initial_gdp);
        let target = (initial_price + next_price) / 2.0;
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: target,
        });
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            gdp: initial_gdp,
            good_prices: vec![("wood".into(), initial_price)],
            ..PlanningParts::default()
        });
        let economy = EconomyContext::new(world, defs, solve_opts);
        let config = SimConfig {
            construction_days: 30,
            default_construction_cost: 30,
            max_added_levels_per_type: 2,
            ..SimConfig::default()
        };

        let gdp_goal = Goal::Simple(SimpleSubgoal::Gdp {
            rel: Rel::Ge,
            value: (initial_gdp + next_gdp) / 2.0,
        });
        let gdp_decisions = successors_with_economy(&state, &gdp_goal, config, &economy);
        assert!(
            gdp_decisions.iter().any(|edge| {
                matches!(
                    &edge.action,
                    Action::QueueBuildingLevel { building, state_id: _ } if building == "building_logging_camp"
                )
            }),
            "gdp goal should queue logging camp; got {gdp_decisions:?}"
        );
        assert!(
            gdp_decisions.iter().any(|edge| {
                matches!(
                    &edge.action,
                    Action::QueueBuildingLevel { building, state_id: _ }
                        if building == BUILDING_CONSTRUCTION_SECTOR
                )
            }),
            "gdp goal should also offer construction sector as capacity lever"
        );

        let decisions = successors_with_economy(&state, &goal, config, &economy);
        let logging = decisions
            .iter()
            .find(|edge| {
                matches!(
                    &edge.action,
                    Action::QueueBuildingLevel { building, state_id: _ } if building == "building_logging_camp"
                )
            })
            .expect("good_price goal should queue logging camp");
        assert_eq!(logging.days, 0);
        let repeated_decision = apply_action(&state, &logging.action, &economy, config).unwrap();
        assert_eq!(repeated_decision, logging.state);
        assert_eq!(repeated_decision.fingerprint(), logging.state.fingerprint());
        let waits = successors_with_economy(&logging.state, &goal, config, &economy);
        // May also offer CS if a parallel slot opens; filter to the wait edge.
        let wait = waits
            .iter()
            .find(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
            .expect("expected construction wait");
        // With remaining=default_construction_cost (30) at rate 1.0 → 30 days.
        assert_eq!(wait.days, 30);
        assert!(matches!(
            wait.action,
            Action::WaitForEvent {
                event: Event::BuildingCompleted { .. },
                days: 30,
            }
        ));
        assert!(evaluate(&goal, &wait.state));
        assert_eq!(wait.state.gdp, next_gdp);
        assert!(evaluate(&gdp_goal, &wait.state));
        let repeated_wait = apply_action(&logging.state, &wait.action, &economy, config).unwrap();
        assert_eq!(repeated_wait, wait.state);
        assert_eq!(repeated_wait.fingerprint(), wait.state.fingerprint());
        assert_eq!(
            waits[0]
                .state
                .building_level_deltas
                .get(&("building_logging_camp".into(), 1)),
            Some(&1)
        );

        let unreachable_gdp = Goal::Simple(SimpleSubgoal::Gdp {
            rel: Rel::Ge,
            value: f64::MAX,
        });
        let mut capped = state;
        for _ in 0..config.max_added_levels_per_type {
            let queue = successors_with_economy(&capped, &unreachable_gdp, config, &economy);
            let logging = queue
                .iter()
                .find(|edge| {
                    matches!(
                        &edge.action,
                        Action::QueueBuildingLevel { building, state_id: _ }
                            if building == "building_logging_camp"
                    )
                })
                .expect("should still offer logging camp under GDP");
            let complete =
                successors_with_economy(&logging.state, &unreachable_gdp, config, &economy);
            let wait = complete
                .iter()
                .find(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
                .expect("expected construction wait after enqueue");
            capped = wait.state.clone();
        }
        assert!(
            successors_with_economy(&capped, &unreachable_gdp, config, &economy)
                .iter()
                .all(|edge| {
                    !matches!(
                        &edge.action,
                        Action::QueueBuildingLevel { building, state_id: _ }
                            if building == "building_logging_camp"
                    )
                }),
            "per-type cap must stop further logging camp levels"
        );
    }

    #[test]
    fn payday_wait_closes_solvent_from_exhausted_credit() {
        let goal = compile("solvent").unwrap();
        let start = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            debt_principal: Some(1_000.0),
            credit_limit: Some(1_000.0),
            credit_headroom: Some(0.0),
            solvent: false,
            weekly_balance: Some(250.0),
            treasury: 0.0,
            ..PlanningParts::default()
        });
        let config = SimConfig {
            payday_days: 7,
            ..SimConfig::default()
        };

        assert!(!evaluate(&goal, &start));
        let waits = successors(&start, &goal, config, &EconomyContext::empty());
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 7);
        assert!(matches!(
            waits[0].action,
            Action::WaitForEvent {
                event: Event::Payday {},
                days: 7,
            }
        ));
        assert!(evaluate(&goal, &waits[0].state));
        assert_eq!(waits[0].state.debt_principal, Some(750.0));
        assert_eq!(waits[0].state.credit_headroom, Some(250.0));
        assert!(waits[0].state.solvent);
        assert_eq!(start.date.days_until(&waits[0].state.date), 7);

        let repeated =
            apply_action(&start, &waits[0].action, &EconomyContext::empty(), config).unwrap();
        assert_eq!(repeated, waits[0].state);
        assert_eq!(repeated.fingerprint(), waits[0].state.fingerprint());
    }

    #[test]
    fn payday_chain_closes_credit_headroom_target() {
        let goal = compile("credit_headroom > 100").unwrap();
        let mut state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            debt_principal: Some(1_000.0),
            credit_limit: Some(1_000.0),
            credit_headroom: Some(0.0),
            solvent: false,
            weekly_balance: Some(80.0),
            treasury: 0.0,
            ..PlanningParts::default()
        });
        let config = SimConfig {
            payday_days: 7,
            ..SimConfig::default()
        };

        for _ in 0..2 {
            assert!(!evaluate(&goal, &state));
            let waits = successors(&state, &goal, config, &EconomyContext::empty());
            assert_eq!(waits.len(), 1);
            state = waits[0].state.clone();
        }
        assert!(evaluate(&goal, &state));
        assert_eq!(state.credit_headroom, Some(160.0));
    }

    #[test]
    fn payday_not_emitted_without_open_solvency_or_progress() {
        let config = SimConfig::default();
        let insolvent = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            debt_principal: Some(1_000.0),
            credit_limit: Some(1_000.0),
            credit_headroom: Some(0.0),
            solvent: false,
            weekly_balance: Some(100.0),
            ..PlanningParts::default()
        });

        let research = successors(
            &insolvent,
            &compile("research(tech=railways)").unwrap(),
            config,
            &EconomyContext::empty(),
        );
        assert!(research.iter().all(|edge| {
            !matches!(
                edge.action,
                Action::WaitForEvent {
                    event: Event::Payday {},
                    ..
                }
            )
        }));

        let deficit = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            debt_principal: Some(1_000.0),
            credit_limit: Some(1_000.0),
            credit_headroom: Some(0.0),
            solvent: false,
            weekly_balance: Some(-50.0),
            treasury: 0.0,
            ..PlanningParts::default()
        });
        assert!(successors(
            &deficit,
            &compile("solvent").unwrap(),
            config,
            &EconomyContext::empty()
        )
        .is_empty());

        let wealth_only = successors_for_simple_subgoals(
            &insolvent,
            &[SimpleSubgoal::PopulationWeightedWealth {
                rel: Rel::Ge,
                value: 20.0,
            }],
            config,
            &EconomyContext::empty(),
        );
        assert!(wealth_only.is_empty());
    }

    #[test]
    fn queue_law_then_wait_reaches_has_law() {
        let goal = compile("has_law(law_homesteading)").unwrap();
        let start = state_at(0);
        let config = SimConfig {
            law_days: 40,
            ..SimConfig::default()
        };

        let decisions = successors(&start, &goal, config, &EconomyContext::empty());
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].days, 0);
        assert!(matches!(
            decisions[0].action,
            Action::QueueLaw { ref law } if law == "law_homesteading"
        ));

        let waits = successors(&decisions[0].state, &goal, config, &EconomyContext::empty());
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 40);
        assert!(waits[0].state.has_law("homesteading"));
        assert!(evaluate(&goal, &waits[0].state));
    }

    #[test]
    fn adjust_tax_closes_weekly_balance() {
        let goal = compile("weekly_balance >= 100").unwrap();
        let start = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            weekly_balance: Some(40.0),
            ..PlanningParts::default()
        });
        let config = SimConfig {
            tax_balance_per_step: 50,
            max_tax_steps: 3,
            ..SimConfig::default()
        };

        let decisions = successors(&start, &goal, config, &EconomyContext::empty());
        assert_eq!(decisions.len(), 1);
        assert!(matches!(
            decisions[0].action,
            Action::AdjustTax { delta: 1, .. }
        ));
        assert_eq!(decisions[0].state.weekly_balance, Some(90.0));
        assert!(!evaluate(&goal, &decisions[0].state));

        let second = successors(&decisions[0].state, &goal, config, &EconomyContext::empty());
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].state.weekly_balance, Some(140.0));
        assert!(evaluate(&goal, &second[0].state));
    }

    #[test]
    fn switch_pm_then_resolves_good_price() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodId::from_usize(0);
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_logging".into()],
                required_construction: Some(250.0),
            },
        );
        defs.production_method_groups.insert(
            "pmg_logging".into(),
            vec!["pm_low".into(), "pm_high".into()],
        );
        defs.production_methods.insert(
            "pm_low".into(),
            ProductionMethod {
                id: "pm_low".into(),
                outputs: vec![(wood, 5.0)],
                ..Default::default()
            },
        );
        defs.production_methods.insert(
            "pm_high".into(),
            ProductionMethod {
                id: "pm_high".into(),
                outputs: vec![(wood, 40.0)],
                ..Default::default()
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_low".into()],
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            }],
            frozen_buy: GoodsVec::from_vec(vec![20.0]),
            ..World::default()
        };
        let solve_opts = SolveOpts::default();
        let baseline = solve(&world, &defs, solve_opts.clone());
        let switched = solve(
            &world.with_production_methods(1, vec!["pm_high".into()]),
            &defs,
            solve_opts.clone(),
        );
        let initial_price = baseline.goods[0].price;
        let next_price = switched.goods[0].price;
        assert!(next_price < initial_price);
        let target = (initial_price + next_price) / 2.0;
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: target,
        });
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            good_prices: vec![("wood".into(), initial_price)],
            gdp: baseline.buildings[0].revenue,
            ..PlanningParts::default()
        });
        let economy = EconomyContext::new(world, defs, solve_opts);
        let config = SimConfig {
            max_added_levels_per_type: 0,
            allow_pm_changes: true,
            ..SimConfig::default()
        };

        let disabled = successors_with_economy(
            &state,
            &goal,
            SimConfig {
                max_added_levels_per_type: 0,
                ..SimConfig::default()
            },
            &economy,
        );
        assert!(
            disabled
                .iter()
                .all(|edge| !matches!(edge.action, Action::SwitchPm { .. })),
            "SwitchPm must be off by default, got {disabled:?}"
        );

        let decisions = successors_with_economy(&state, &goal, config, &economy);
        assert!(
            decisions.iter().any(|edge| matches!(
                edge.action,
                Action::SwitchPm {
                    building_id: 1,
                    ref methods
                } if methods == &["pm_high".to_string()]
            )),
            "expected SwitchPm candidate, got {decisions:?}"
        );
        let switched_edge = decisions
            .into_iter()
            .find(|edge| matches!(edge.action, Action::SwitchPm { building_id: 1, .. }))
            .expect("switch pm edge");
        assert_eq!(switched_edge.days, 0);
        assert!(evaluate(&goal, &switched_edge.state));
        assert_eq!(
            switched_edge.state.pm_overrides.get(&1).map(Vec::as_slice),
            Some(["pm_high".to_string()].as_slice())
        );
    }

    #[test]
    fn shops_for_planning_matches_from_world_projection() {
        use vic3_defs::{BuildingType, ProductionMethod};
        use vic3_prices::equilibrate;

        let wood = GoodId::from_usize(0);
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_logging".into()],
                required_construction: Some(30.0),
            },
        );
        defs.production_method_groups.insert(
            "pmg_logging".into(),
            vec!["pm_sawmills".into(), "pm_high".into()],
        );
        defs.production_methods.insert(
            "pm_sawmills".into(),
            ProductionMethod {
                id: "pm_sawmills".into(),
                outputs: vec![(wood, 10.0)],
                ..Default::default()
            },
        );
        defs.production_methods.insert(
            "pm_high".into(),
            ProductionMethod {
                id: "pm_high".into(),
                outputs: vec![(wood, 25.0)],
                ..Default::default()
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                infrastructure: Some(10.0),
                infrastructure_usage: Some(0.0),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_sawmills".into()],
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let mut state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        state
            .building_level_deltas
            .insert(("building_logging_camp".into(), 1), 1);
        state.pm_overrides.insert(1, vec!["pm_high".into()]);

        let patched = economy.shops_for_planning(&state);
        let projected = economy.apply_planning_to_world(&state);
        let rebuilt = ShopCache::from_world(&projected, &economy.defs);

        for (good, qty) in patched.frozen_sell.iter_indexed() {
            assert!(
                (qty - rebuilt.frozen_sell[good]).abs() < 1e-9,
                "market sell mismatch good {good:?}"
            );
        }
        for (good, qty) in patched.frozen_buy.iter_indexed() {
            assert!(
                (qty - rebuilt.frozen_buy[good]).abs() < 1e-9,
                "market buy mismatch good {good:?}"
            );
        }
        let shop = patched.shops.iter().find(|s| s.id == 1).unwrap();
        let rebuilt_shop = rebuilt.shops.iter().find(|s| s.id == 1).unwrap();
        for (good, qty) in shop.frozen_sell.iter_indexed() {
            assert!(
                (qty - rebuilt_shop.frozen_sell[good]).abs() < 1e-9,
                "state sell mismatch good {good:?}"
            );
        }

        let from_patch = equilibrate_cached(&patched, &economy.defs, SolveOpts::default());
        let from_world = equilibrate(&projected, &economy.defs, SolveOpts::default());
        assert_eq!(from_patch.goods.len(), from_world.goods.len());
        for (a, b) in from_patch.goods.iter().zip(from_world.goods.iter()) {
            assert_eq!(a.name, b.name);
            assert!(
                (a.price - b.price).abs() < 1e-9,
                "price mismatch for {}: {} vs {}",
                a.name,
                a.price,
                b.price
            );
        }
    }

    #[test]
    fn construction_remaining_over_rate_ceils_wait_days() {
        use crate::world::{ConstructionQueueKind, PlanningConstruction};

        let defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_building: Some("building_logging_camp".into()),
            constructions: vec![PlanningConstruction {
                order_id: 1,
                queue: ConstructionQueueKind::Government,
                state_id: None,
                building: "building_logging_camp".into(),
                remaining: Some(25.0),
            }],
            construction_points_per_day: 10.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 0.0,
        });
        let waits = successors_with_economy(
            &state,
            &goal,
            SimConfig {
                max_construction_allocation: Some(1000),
                ..SimConfig::default()
            },
            &economy,
        );
        assert_eq!(waits.len(), 1);
        // ceil(25/10) = 3
        assert_eq!(waits[0].days, 3);
        assert!(matches!(
            waits[0].action,
            Action::WaitForEvent {
                event: Event::BuildingCompleted { .. },
                days: 3,
            }
        ));
    }

    #[test]
    fn new_build_uses_def_required_construction_for_wait() {
        use vic3_defs::BuildingType;

        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: Vec::new(),
                required_construction: Some(25.0),
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            construction_points_per_day: 10.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let config = SimConfig {
            // Large fallback so a wrong path would not accidentally match ceil(25/10).
            construction_days: 180,
            max_construction_allocation: Some(1000),
            ..SimConfig::default()
        };
        let queued = apply_action_with_economy(
            &state,
            &Action::QueueBuildingLevel {
                building: "building_logging_camp".into(),
                state_id: 1,
            },
            &economy,
            config,
        )
        .expect("enqueue");
        assert_eq!(
            queued.constructions.first().and_then(|row| row.remaining),
            Some(25.0)
        );
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 0.0,
        });
        let waits = successors_with_economy(&queued, &goal, config, &economy);
        assert_eq!(waits.len(), 1);
        // ceil(25/10) = 3
        assert_eq!(waits[0].days, 3);
        assert!(matches!(
            waits[0].action,
            Action::WaitForEvent {
                event: Event::BuildingCompleted { .. },
                days: 3,
            }
        ));
    }

    #[test]
    fn in_flight_remaining_wins_over_def_required_construction() {
        use crate::world::{ConstructionQueueKind, PlanningConstruction};
        use vic3_defs::BuildingType;

        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: Vec::new(),
                // Would imply ceil(999/10)=100 if wrongly preferred over remaining.
                required_construction: Some(999.0),
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_building: Some("building_logging_camp".into()),
            constructions: vec![PlanningConstruction {
                order_id: 1,
                queue: ConstructionQueueKind::Government,
                state_id: None,
                building: "building_logging_camp".into(),
                remaining: Some(25.0),
            }],
            construction_points_per_day: 10.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 0.0,
        });
        let waits = successors_with_economy(
            &state,
            &goal,
            SimConfig {
                max_construction_allocation: Some(1000),
                ..SimConfig::default()
            },
            &economy,
        );
        assert_eq!(waits.len(), 1);
        // ceil(25/10) = 3; def cost 999 must not win
        assert_eq!(waits[0].days, 3);
        assert!(matches!(
            waits[0].action,
            Action::WaitForEvent {
                event: Event::BuildingCompleted { .. },
                days: 3,
            }
        ));
    }

    #[test]
    fn research_and_construction_run_in_parallel_not_serialized() {
        use crate::world::{ConstructionQueueKind, PlanningConstruction};

        let defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let config = SimConfig {
            research_days: 100,
            construction_days: 180,
            ..SimConfig::default()
        };
        let mut state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_tech: Some("nitroglycerin".into()),
            tech_days_left: Some(100),
            queued_building: Some("building_logging_camp".into()),
            constructions: vec![PlanningConstruction {
                order_id: 1,
                queue: ConstructionQueueKind::Government,
                state_id: None,
                building: "building_logging_camp".into(),
                remaining: Some(50.0),
            }],
            construction_points_per_day: 1.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let start_date = state.date;
        let goal = Goal::And(vec![
            Goal::Simple(SimpleSubgoal::HasTech("nitroglycerin".into())),
            Goal::Simple(SimpleSubgoal::GoodPrice {
                good: "wood".into(),
                rel: Rel::Le,
                value: 0.0,
            }),
        ]);

        // Earliest wait is construction (50), not research+construction (150).
        let first = successors_with_economy(&state, &goal, config, &economy);
        let first_wait = first
            .iter()
            .find(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
            .expect("expected a construction wait");
        assert_eq!(first_wait.days, 50);
        assert!(matches!(
            first_wait.action,
            Action::WaitForEvent {
                event: Event::BuildingCompleted { .. },
                days: 50,
            }
        ));
        state = first_wait.state.clone();
        assert!(state.queued_tech.as_deref() == Some("nitroglycerin"));
        assert_eq!(state.tech_days_left, Some(50));
        assert!(state.queued_building.is_none());

        let second = successors_with_economy(&state, &goal, config, &economy);
        let second_wait = second
            .iter()
            .find(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
            .expect("expected a tech wait");
        assert_eq!(second_wait.days, 50);
        assert!(matches!(
            second_wait.action,
            Action::WaitForEvent {
                event: Event::TechCompleted { .. },
                days: 50,
            }
        ));
        assert!(second_wait.state.has_tech("nitroglycerin"));
        assert_eq!(start_date.days_until(&second_wait.state.date), 100);
    }

    #[test]
    fn higher_cs_capacity_shortens_construction_wait() {
        use crate::world::{ConstructionQueueKind, PlanningConstruction};

        let defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 0.0,
        });
        let job = || PlanningConstruction {
            order_id: 1,
            queue: ConstructionQueueKind::Government,
            state_id: None,
            building: "building_logging_camp".into(),
            remaining: Some(100.0),
        };
        let slow = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_building: Some("building_logging_camp".into()),
            constructions: vec![job()],
            construction_points_per_day: 5.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let fast = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_building: Some("building_logging_camp".into()),
            constructions: vec![job()],
            construction_points_per_day: 20.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let config = SimConfig {
            max_construction_allocation: Some(1000),
            ..SimConfig::default()
        };
        let slow_days = construction_wait_days(&slow, config).unwrap();
        let fast_days = construction_wait_days(&fast, config).unwrap();
        assert_eq!(slow_days, 20); // ceil(100/5)
        assert_eq!(fast_days, 5); // ceil(100/20)
        assert!(fast_days < slow_days);
        let _ = (&economy, &goal);
    }

    #[test]
    fn parallel_allocation_advances_two_jobs_under_capacity_cap() {
        use crate::world::{ConstructionQueueKind, PlanningConstruction};

        let defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let config = SimConfig {
            max_construction_allocation: Some(5),
            ..SimConfig::default()
        };
        // Capacity 10 / alloc 5 → two parallel slots.
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_building: Some("building_a".into()),
            constructions: vec![
                PlanningConstruction {
                    order_id: 1,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_a".into(),
                    remaining: Some(50.0),
                },
                PlanningConstruction {
                    order_id: 2,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_b".into(),
                    remaining: Some(50.0),
                },
            ],
            construction_points_per_day: 10.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        assert_eq!(max_parallel_construction_jobs(10.0, 5.0).get(), 2);
        // Each job gets 5/day → ceil(50/5)=10 days to first completion.
        assert_eq!(construction_wait_days(&state, config), Some(10));
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 0.0,
        });
        let waits = successors_with_economy(&state, &goal, config, &economy);
        let wait = waits
            .iter()
            .find(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
            .expect("wait");
        assert_eq!(wait.days, 10);
        // After completing one, the other should have 0 remaining (also finished).
        assert!(wait.state.constructions.len() <= 1);
        if let Some(left) = wait.state.constructions.first() {
            assert!(
                left.remaining.is_some_and(|r| r <= 0.0 + 1e-9),
                "peer job should be drained in parallel: {left:?}"
            );
        }
    }

    #[test]
    fn completing_construction_sector_raises_capacity() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            BUILDING_CONSTRUCTION_SECTOR.into(),
            BuildingType {
                id: BUILDING_CONSTRUCTION_SECTOR.into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_base_building_construction_sector".into()],
                required_construction: Some(10.0),
            },
        );
        defs.production_method_groups.insert(
            "pmg_base_building_construction_sector".into(),
            vec!["pm_iron_frame_buildings".into()],
        );
        defs.production_methods.insert(
            "pm_iron_frame_buildings".into(),
            ProductionMethod {
                id: "pm_iron_frame_buildings".into(),
                country_construction_add: Some(5.0),
                ..ProductionMethod::default()
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![
                WorldBuilding {
                    id: 1,
                    state: Some(1),
                    building: "building_logging_camp".into(),
                    level: 1.0,
                    staffing: 1.0,
                    production_methods: Vec::new(),
                    saved_inputs: Vec::new(),
                    saved_outputs: vec![(GoodId::from_usize(0), 10.0)],
                },
                WorldBuilding {
                    id: 2,
                    state: Some(1),
                    building: BUILDING_CONSTRUCTION_SECTOR.into(),
                    level: 0.0,
                    staffing: 0.0,
                    production_methods: vec!["pm_iron_frame_buildings".into()],
                    saved_inputs: Vec::new(),
                    saved_outputs: Vec::new(),
                },
            ],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let config = SimConfig {
            base_construction_capacity: 1,
            default_construction_cost: 10,
            ..SimConfig::default()
        };
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            construction_points_per_day: 1.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let queued = apply_action_with_economy(
            &state,
            &Action::QueueBuildingLevel {
                building: BUILDING_CONSTRUCTION_SECTOR.into(),
                state_id: 1,
            },
            &economy,
            config,
        )
        .expect("queue CS");
        let done = apply_action_with_economy(
            &queued,
            &Action::WaitForEvent {
                event: Event::BuildingCompleted {
                    building: BUILDING_CONSTRUCTION_SECTOR.into(),
                    state_id: Some(1),
                },
                days: 10,
            },
            &economy,
            config,
        )
        .expect("complete CS");
        assert_eq!(
            done.building_level_deltas
                .get(&(BUILDING_CONSTRUCTION_SECTOR.into(), 1)),
            Some(&1)
        );
        // base 1 + 5 per CS level
        assert!((done.construction_points_per_day - 6.0).abs() < 1e-9);
    }

    /// Tiny local GER world with logging + CS for speculative tests (not shared fixtures).
    fn speculative_logging_cs_mini() -> (PlanningState, EconomyContext, SimConfig, u32) {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodId::from_usize(0);
        let state_id = 1u32;
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_logging".into()],
                required_construction: Some(10.0),
            },
        );
        defs.production_method_groups
            .insert("pmg_logging".into(), vec!["pm_sawmills".into()]);
        defs.production_methods.insert(
            "pm_sawmills".into(),
            ProductionMethod {
                id: "pm_sawmills".into(),
                outputs: vec![(wood, 10.0)],
                ..ProductionMethod::default()
            },
        );
        defs.buildings.insert(
            BUILDING_CONSTRUCTION_SECTOR.into(),
            BuildingType {
                id: BUILDING_CONSTRUCTION_SECTOR.into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_base_building_construction_sector".into()],
                required_construction: Some(10.0),
            },
        );
        defs.production_method_groups.insert(
            "pmg_base_building_construction_sector".into(),
            vec!["pm_iron_frame_buildings".into()],
        );
        defs.production_methods.insert(
            "pm_iron_frame_buildings".into(),
            ProductionMethod {
                id: "pm_iron_frame_buildings".into(),
                country_construction_add: Some(5.0),
                ..ProductionMethod::default()
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: state_id,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![
                WorldBuilding {
                    id: 1,
                    state: Some(state_id),
                    building: "building_logging_camp".into(),
                    level: 1.0,
                    staffing: 1.0,
                    production_methods: vec!["pm_sawmills".into()],
                    saved_inputs: Vec::new(),
                    saved_outputs: vec![(wood, 10.0)],
                },
                WorldBuilding {
                    id: 2,
                    state: Some(state_id),
                    building: BUILDING_CONSTRUCTION_SECTOR.into(),
                    level: 0.0,
                    staffing: 0.0,
                    production_methods: vec!["pm_iron_frame_buildings".into()],
                    saved_inputs: Vec::new(),
                    saved_outputs: Vec::new(),
                },
            ],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let config = SimConfig {
            base_construction_capacity: 1,
            default_construction_cost: 10,
            ..SimConfig::default()
        };
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            gdp: 1.0,
            construction_points_per_day: 1.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        (state, economy, config, state_id)
    }

    #[test]
    fn speculative_completed_state_finishes_cs_and_raises_points() {
        let (state, economy, config, state_id) = speculative_logging_cs_mini();
        let done = speculative_completed_state(
            &state,
            &Action::QueueBuildingLevel {
                building: BUILDING_CONSTRUCTION_SECTOR.into(),
                state_id,
            },
            &economy,
            config,
        )
        .expect("speculative complete CS");
        assert_eq!(
            done.building_level_deltas
                .get(&(BUILDING_CONSTRUCTION_SECTOR.into(), state_id)),
            Some(&1)
        );
        // base 1 + 5 per CS level
        assert!((done.construction_points_per_day - 6.0).abs() < 1e-9);
        assert!(done.gdp.is_finite());
    }

    #[test]
    fn speculative_completed_state_finishes_logging_and_refreshes_gdp() {
        let (state, economy, config, state_id) = speculative_logging_cs_mini();
        let done = speculative_completed_state(
            &state,
            &Action::QueueBuildingLevel {
                building: "building_logging_camp".into(),
                state_id,
            },
            &economy,
            config,
        )
        .expect("speculative complete logging");
        assert_eq!(
            done.building_level_deltas
                .get(&("building_logging_camp".into(), state_id)),
            Some(&1)
        );
        assert!(done.gdp.is_finite());
        assert_ne!(done.gdp, state.gdp);
    }

    #[test]
    fn speculative_completed_state_rejects_unapplicable_action() {
        let (state, economy, config, _) = speculative_logging_cs_mini();
        let err = speculative_completed_state(
            &state,
            &Action::QueueTech {
                tech: String::new(),
            },
            &economy,
            config,
        )
        .expect_err("empty tech must fail");
        assert_eq!(err, SpeculativeCompleteError::ApplyRejected);
    }

    #[test]
    fn gdp_goal_can_queue_construction_sector_as_means_to_an_end() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodId::from_usize(0);
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_logging".into()],
                required_construction: Some(100.0),
            },
        );
        defs.production_method_groups
            .insert("pmg_logging".into(), vec!["pm_sawmills".into()]);
        defs.production_methods.insert(
            "pm_sawmills".into(),
            ProductionMethod {
                id: "pm_sawmills".into(),
                outputs: vec![(wood, 10.0)],
                ..Default::default()
            },
        );
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_sawmills".into()],
                saved_inputs: Vec::new(),
                saved_outputs: vec![(wood, 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            gdp: 1.0,
            good_prices: vec![("wood".into(), 20.0)],
            ..PlanningParts::default()
        });
        let goal = Goal::Simple(SimpleSubgoal::Gdp {
            rel: Rel::Ge,
            value: 1.0e12,
        });
        let edges = successors_with_economy(&state, &goal, SimConfig::default(), &economy);
        assert!(
            edges.iter().any(|edge| {
                matches!(
                    &edge.action,
                    Action::QueueBuildingLevel { building, state_id: _ }
                        if building == BUILDING_CONSTRUCTION_SECTOR
                )
            }),
            "expected CS means-to-an-end edge among {edges:?}"
        );
    }

    #[test]
    fn first_of_type_building_candidate_and_greenfield_prices() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodId::from_usize(0);
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.buildings.insert(
            "building_logging_camp".into(),
            BuildingType {
                id: "building_logging_camp".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_logging".into()],
                required_construction: Some(30.0),
            },
        );
        defs.production_method_groups
            .insert("pmg_logging".into(), vec!["pm_sawmills".into()]);
        defs.production_methods.insert(
            "pm_sawmills".into(),
            ProductionMethod {
                id: "pm_sawmills".into(),
                outputs: vec![(wood, 20.0)],
                ..Default::default()
            },
        );
        // No logging camp in the world — first-of-type.
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            buildings: Vec::new(),
            frozen_buy: GoodsVec::from_vec(vec![30.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            good_prices: vec![("wood".into(), 35.0)],
            gdp: 0.0,
            ..PlanningParts::default()
        });
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 10.0,
        });
        let config = SimConfig {
            default_construction_cost: 30,
            ..SimConfig::default()
        };
        let edges = successors_with_economy(&state, &goal, config, &economy);
        assert!(
            edges.iter().any(|edge| {
                matches!(
                    &edge.action,
                    Action::QueueBuildingLevel { building, state_id: _ } if building == "building_logging_camp"
                )
            }),
            "first-of-type logging camp must be a candidate: {edges:?}"
        );

        let queued = apply_action_with_economy(
            &state,
            &Action::QueueBuildingLevel {
                building: "building_logging_camp".into(),
                state_id: 1,
            },
            &economy,
            config,
        )
        .expect("enqueue first-of-type");
        let completed = apply_action_with_economy(
            &queued,
            &Action::WaitForEvent {
                event: Event::BuildingCompleted {
                    building: "building_logging_camp".into(),
                    state_id: Some(1),
                },
                days: construction_wait_days(&queued, config).expect("wait"),
            },
            &economy,
            config,
        )
        .expect("complete first-of-type");
        assert_eq!(
            completed
                .building_level_deltas
                .get(&("building_logging_camp".into(), 1))
                .copied(),
            Some(1)
        );
        let price = completed.price("wood").expect("price after greenfield");
        assert!(
            price < 35.0,
            "greenfield completion should move wood price, got {price}"
        );
    }

    #[test]
    fn building_candidates_enqueue_by_state_no_dominance_prune() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodId::from_usize(0);
        let mut defs = GameDefs {
            goods_order: vec!["wood".into()],
            goods: BTreeMap::from([(
                "wood".into(),
                Good {
                    id: "wood".into(),
                    base_price: 20.0,
                    traded_quantity: 10.0,
                    texture: None,
                },
            )]),
            price_range: 0.75,
            ..GameDefs::default()
        };
        for (id, cost, out) in [
            ("building_good_mill", 50.0, 10.0),
            ("building_expensive_mill", 200.0, 10.0),
            ("building_weak_mill", 50.0, 2.0),
        ] {
            defs.buildings.insert(
                id.into(),
                BuildingType {
                    id: id.into(),
                    group: None,
                    city_type: None,
                    production_method_groups: vec![format!("pmg_{id}")],
                    required_construction: Some(cost),
                },
            );
            defs.production_method_groups
                .insert(format!("pmg_{id}"), vec![format!("pm_{id}")]);
            defs.production_methods.insert(
                format!("pm_{id}"),
                ProductionMethod {
                    id: format!("pm_{id}"),
                    outputs: vec![(wood, out)],
                    ..Default::default()
                },
            );
        }
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![
                WorldState {
                    id: 10,
                    country: Some(1),
                    ..WorldState::default()
                },
                WorldState {
                    id: 20,
                    country: Some(1),
                    ..WorldState::default()
                },
            ],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(10),
                building: "building_good_mill".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_building_good_mill".into()],
                saved_inputs: Vec::new(),
                saved_outputs: vec![(wood, 10.0)],
            }],
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            good_prices: vec![("wood".into(), 40.0)],
            ..PlanningParts::default()
        });
        let goal = Goal::Simple(SimpleSubgoal::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 1.0,
        });
        let edges = successors_with_economy(&state, &goal, SimConfig::default(), &economy);
        let queued: BTreeSet<_> = edges
            .iter()
            .filter_map(|edge| match &edge.action {
                Action::QueueBuildingLevel { building, state_id } => {
                    Some((building.as_str(), *state_id))
                }
                _ => None,
            })
            .collect();
        assert!(
            queued.contains(&("building_good_mill", 10)),
            "existing mill expands in its state: {queued:?}"
        );
        assert!(
            !queued
                .iter()
                .any(|(b, s)| *b == "building_good_mill" && *s == 20),
            "do not greenfield good_mill in other states while one exists: {queued:?}"
        );
        assert!(
            queued.contains(&("building_expensive_mill", 10))
                && queued.contains(&("building_expensive_mill", 20)),
            "no dominance prune — expensive mill offered in owned states: {queued:?}"
        );
        assert!(
            queued.contains(&("building_weak_mill", 10))
                && queued.contains(&("building_weak_mill", 20)),
            "no dominance prune — weak mill offered in owned states: {queued:?}"
        );
    }

    #[test]
    fn mil_queue_building_ok_while_research_busy() {
        let defs = GameDefs::default();
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let start = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_tech: Some("railways".into()),
            tech_days_left: Some(200),
            army_power_projection: Some(0.0),
            army_pp_baseline: Some(0.0),
            ..PlanningParts::default()
        });
        let goal = compile("army_power_projection >= 10").unwrap();
        let edges = successors_with_economy(&start, &goal, SimConfig::default(), &economy);
        assert!(
            edges.iter().any(|edge| {
                matches!(
                    &edge.action,
                    Action::QueueBuildingLevel { building, state_id: _ } if building == BUILDING_BARRACKS
                )
            }),
            "military construction must not be blocked by research: {edges:?}"
        );
        assert!(apply_action_with_economy(
            &start,
            &Action::QueueBuildingLevel {
                building: BUILDING_BARRACKS.into(),
                state_id: 1,
            },
            &economy,
            SimConfig::default(),
        )
        .is_some());
        assert!(
            apply_action(
                &start,
                &Action::QueueTech {
                    tech: "nitroglycerin".into(),
                },
                &EconomyContext::empty(),
                SimConfig::default(),
            )
            .is_none(),
            "second research enqueue must still respect research track"
        );
    }

    #[test]
    fn wait_zero_days_completes_when_timer_elapsed() {
        let start = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_tech: Some("railways".into()),
            tech_days_left: Some(0),
            ..PlanningParts::default()
        });
        let wait = Action::WaitForEvent {
            event: Event::TechCompleted {
                tech: "railways".into(),
            },
            days: 0,
        };
        let next = apply_action(
            &start,
            &wait,
            &EconomyContext::empty(),
            SimConfig::default(),
        )
        .unwrap();
        assert!(next.has_tech("railways"));
        assert!(next.queued_tech.is_none());
        assert!(next.tech_days_left.is_none());
        assert_eq!(next.date, start.date);

        let rejected = Action::WaitForEvent {
            event: Event::TechCompleted {
                tech: "railways".into(),
            },
            days: 0,
        };
        let not_ready = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_tech: Some("railways".into()),
            tech_days_left: Some(5),
            ..PlanningParts::default()
        });
        assert!(apply_action(
            &not_ready,
            &rejected,
            &EconomyContext::empty(),
            SimConfig::default()
        )
        .is_none());
    }

    #[test]
    fn tech_only_plan_cost_unchanged_after_parallel_tracks() {
        use crate::plan::plan;

        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        let result = plan(
            state,
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
            EconomyContext::empty(),
            1000,
            0.0,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(result.day_cost, 365);
        assert_eq!(result.actions.len(), 2);
    }

    #[test]
    fn i8_actions_deterministic_and_hash_stable() {
        let state = state_at(7);
        let config = SimConfig::default();
        let action = Action::QueueTech {
            tech: "railways".into(),
        };
        let a = apply_action(&state, &action, &EconomyContext::empty(), config).unwrap();
        let b = apply_action(
            &state.clone(),
            &action.clone(),
            &EconomyContext::empty(),
            config,
        )
        .unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());

        let wait = Action::WaitForEvent {
            event: Event::TechCompleted {
                tech: "railways".into(),
            },
            days: 45,
        };
        let a = apply_action(&a, &wait, &EconomyContext::empty(), config).unwrap();
        let b = apply_action(&b, &wait, &EconomyContext::empty(), config).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// I6: successor dates are monotone, and idle non-solvency goals have
        /// no event-wait edge.
        #[test]
        fn i6_event_wait_date_monotone_and_not_spurious(
            day_offset in 0i32..20_000,
            research_days in any::<u16>(),
            queue_tech in any::<bool>(),
        ) {
            let goal = compile("research(tech=railways)").unwrap();
            let mut state = state_at(day_offset);
            if queue_tech {
                state.queued_tech = Some("railways".into());
            }
            let config = SimConfig {
                research_days,
                ..SimConfig::default()
            };
            let edges = successors(&state, &goal, config, &EconomyContext::empty());

            let wait_count = edges
                .iter()
                .filter(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
                .count();
            prop_assert!(wait_count <= 1);
            for edge in &edges {
                prop_assert!(edge.state.date >= state.date);
                if let Action::WaitForEvent { days, .. } = &edge.action {
                    if *days > 0 {
                        prop_assert!(edge.state.date > state.date);
                    } else {
                        prop_assert_eq!(edge.state.date, state.date);
                    }
                }
            }

            if !queue_tech {
                prop_assert_eq!(wait_count, 0);
            }

            let idle_atoms = [SimpleSubgoal::Solvent];
            let idle_edges = successors_for_simple_subgoals(
                &state_at(day_offset),
                &idle_atoms,
                config,
            &EconomyContext::empty(),
            );
            let has_wait = idle_edges
                .iter()
                .any(|edge| matches!(&edge.action, Action::WaitForEvent { .. }));
            prop_assert!(!has_wait);

            let solvent_start = PlanningState::from_parts(PlanningParts {
                date: Vic3Date::from_ymdh(1836, 1, 1, 0).add_days(day_offset),
                country: "GER".into(),
                debt_principal: Some(500.0),
                credit_limit: Some(500.0),
                credit_headroom: Some(0.0),
                solvent: false,
                weekly_balance: Some(25.0),
                ..PlanningParts::default()
            });
            let solvent_edges = successors(
                &solvent_start,
                &compile("solvent").unwrap(),
                config,
            &EconomyContext::empty(),
            );
            prop_assert_eq!(solvent_edges.len(), 1);
            let is_payday = matches!(
                solvent_edges[0].action,
                Action::WaitForEvent {
                    event: Event::Payday {},
                    ..
                }
            );
            prop_assert!(is_payday);
            prop_assert!(solvent_edges[0].state.date > solvent_start.date);
        }
    }
}
