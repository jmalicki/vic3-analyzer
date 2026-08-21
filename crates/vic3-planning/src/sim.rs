//! Goal-relevant simulator successors over [`crate::world::PlanningState`].
//!
//! # Why goal-relevant only
//!
//! Enumerating the whole game is intractable. [`successors`] / [`successors_for_atoms`]
//! open only edges that can close currently failing [`crate::goals::Atom`]s (plus
//! capped building / PM candidates when an [`EconomyContext`] is present). Idle
//! atoms with no model action emit nothing.
//!
//! # Edges
//!
//! - **Decision** (0 days): queue tech/building/interest/army/law, `SwitchPm`,
//!   `AdjustTax`. Each track has its own occupancy check (research vs
//!   construction may run in parallel). Instant decisions are not blocked by
//!   other tracks' waits.
//! - **At most one event-wait** per expansion: the earliest completion among
//!   goal-relevant in-flight tracks (or payday). Other tracks' timers tick in
//!   parallel via [`PlanningState::tick_parallel_tracks`]. **I6:** wait never
//!   decreases date.
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

use crate::goals::{gaps, Atom, Goal, InterestKind, Rel};
use crate::military::{
    is_barracks_building, is_military_planning_building, is_naval_admin_building,
    is_shipyard_building, UnitCombatStats, BUILDING_BARRACKS, BUILDING_NAVAL_ADMIN,
    BUILDING_SHIPYARD, MIL_INPUT_PRICE_FACTOR,
};
use crate::world::{PlanningState, QueuedInterest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use vic3_defs::GameDefs;
use vic3_prices::{solve, PricesResult, SolveOpts, World, ORDER_EPS};

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
    economy: Option<&'a EconomyContext>,
    config: SimConfig,
    seen_builds: &'a mut BTreeSet<String>,
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
        // Still may need to hire underemployed buildings so the atom can clear.
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
    if push_mil_hire_decisions(args, branch) {
        return;
    }
    let Some(economy) = args.economy else {
        return;
    };
    let unit = match branch {
        MilitaryBranch::Army => UnitCombatStats::army_default(),
        MilitaryBranch::Navy => UnitCombatStats::navy_default(),
    };
    let per = unit.full_power_projection().max(1.0);
    let levels_needed = (needed / per).ceil().max(1.0) as u32;
    let cap = u32::from(args.config.max_added_levels_per_type);
    match branch {
        MilitaryBranch::Army => {
            let have = mil_levels(args.state, is_barracks_building);
            if have < levels_needed.min(cap) && args.seen_builds.insert(BUILDING_BARRACKS.into()) {
                push_decision(
                    args.result,
                    args.state,
                    Action::QueueBuildingLevel {
                        building: BUILDING_BARRACKS.into(),
                    },
                    Some(economy),
                    args.config,
                );
            }
        }
        MilitaryBranch::Navy => {
            let ships_needed = levels_needed.min(cap);
            let shipyard = mil_levels(args.state, is_shipyard_building);
            let admin = mil_levels(args.state, is_naval_admin_building);
            if shipyard < ships_needed && args.seen_builds.insert(BUILDING_SHIPYARD.into()) {
                push_decision(
                    args.result,
                    args.state,
                    Action::QueueBuildingLevel {
                        building: BUILDING_SHIPYARD.into(),
                    },
                    Some(economy),
                    args.config,
                );
            }
            if admin < ships_needed && args.seen_builds.insert(BUILDING_NAVAL_ADMIN.into()) {
                push_decision(
                    args.result,
                    args.state,
                    Action::QueueBuildingLevel {
                        building: BUILDING_NAVAL_ADMIN.into(),
                    },
                    Some(economy),
                    args.config,
                );
            }
        }
    }
}

fn mil_levels(state: &PlanningState, pred: fn(&str) -> bool) -> u32 {
    state
        .mil_buildings
        .iter()
        .filter(|b| pred(&b.building))
        .map(|b| b.levels.floor() as u32)
        .sum()
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
            *economy,
            *config,
        );
        pushed = true;
    }
    pushed
}

/// Immutable price-solver inputs shared by all nodes in one search.
#[derive(Debug, Clone)]
pub struct EconomyContext {
    pub base_world: World,
    pub defs: GameDefs,
    pub solve_opts: SolveOpts,
}

impl EconomyContext {
    pub fn new(base_world: World, defs: GameDefs, solve_opts: SolveOpts) -> Self {
        Self {
            base_world,
            defs,
            solve_opts,
        }
    }

    fn world_for(&self, state: &PlanningState) -> World {
        let with_levels = state
            .building_level_deltas
            .iter()
            .fold(self.base_world.clone(), |world, (building, levels)| {
                world.with_extra_levels(building, *levels)
            });
        state
            .pm_overrides
            .iter()
            .fold(with_levels, |world, (building_id, methods)| {
                world.with_production_methods(*building_id, methods.clone())
            })
    }

    fn building_candidates(&self, state: &PlanningState, atoms: &[Atom], cap: u16) -> Vec<String> {
        let world = self.world_for(state);
        let mut candidates = BTreeSet::new();
        for atom in atoms {
            let Atom::GoodPrice { good, rel, .. } = atom else {
                continue;
            };
            for building in &world.buildings {
                if state
                    .building_level_deltas
                    .get(&building.building)
                    .copied()
                    .unwrap_or(0)
                    >= u32::from(cap)
                {
                    continue;
                }
                let (inputs, outputs) = building.goods_io(&self.defs);
                let Some(good_idx) = self.defs.index_of(good) else {
                    continue;
                };
                let produces = outputs[good_idx] > ORDER_EPS;
                let consumes = inputs[good_idx] > ORDER_EPS;
                let relevant = match rel {
                    Rel::Le | Rel::Lt => produces,
                    Rel::Ge | Rel::Gt => consumes,
                    Rel::Eq => produces || consumes,
                };
                if relevant {
                    candidates.insert(building.building.clone());
                }
            }
        }
        if atoms.iter().any(|atom| {
            matches!(
                atom,
                Atom::Gdp {
                    rel: Rel::Ge | Rel::Gt | Rel::Eq,
                    ..
                }
            )
        }) {
            let mut scores = BTreeMap::<String, f64>::new();
            for building in &world.buildings {
                if state
                    .building_level_deltas
                    .get(&building.building)
                    .copied()
                    .unwrap_or(0)
                    >= u32::from(cap)
                {
                    continue;
                }
                let (_, outputs) = building.goods_io(&self.defs);
                let per_level = building.level.max(1.0);
                let score = outputs
                    .iter_indexed()
                    .filter(|(_, quantity)| *quantity > ORDER_EPS)
                    .map(|(good, quantity)| {
                        let price = self.defs.good_by_index(good).and_then(|id| {
                            state
                                .price(id)
                                .or_else(|| self.defs.goods.get(id).map(|g| g.base_price))
                        });
                        price.unwrap_or(0.0) * quantity.max(0.0) / per_level
                    })
                    .sum::<f64>();
                *scores.entry(building.building.clone()).or_default() += score;
            }
            let mut ranked: Vec<_> = scores
                .into_iter()
                .filter(|(_, score)| score.is_finite() && *score > 0.0)
                .collect();
            ranked.sort_by(|(left_id, left), (right_id, right)| {
                right.total_cmp(left).then_with(|| left_id.cmp(right_id))
            });
            candidates.extend(ranked.into_iter().take(3).map(|(building, _)| building));
        }
        // When army/navy PP is open, queue producers for expensive mil/shipyard inputs.
        if atoms
            .iter()
            .any(|atom| matches!(atom, Atom::ArmyPower { .. } | Atom::NavyPower { .. }))
        {
            self.add_mil_input_producer_candidates(state, &world, &mut candidates, cap);
        }
        candidates.into_iter().collect()
    }

    fn add_mil_input_producer_candidates(
        &self,
        state: &PlanningState,
        world: &World,
        candidates: &mut BTreeSet<String>,
        cap: u16,
    ) {
        let mil_types: Vec<&str> = world
            .buildings
            .iter()
            .map(|b| b.building.as_str())
            .chain([BUILDING_BARRACKS, BUILDING_SHIPYARD, BUILDING_NAVAL_ADMIN])
            .filter(|id| is_military_planning_building(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut expensive_inputs = BTreeSet::new();
        for building in &world.buildings {
            if !is_military_planning_building(&building.building) {
                continue;
            }
            let (inputs, _) = building.goods_io(&self.defs);
            for (good_idx, qty) in inputs.iter_indexed() {
                if qty <= ORDER_EPS {
                    continue;
                }
                let Some(good_id) = self.defs.good_by_index(good_idx) else {
                    continue;
                };
                let Some(def) = self.defs.goods.get(good_id) else {
                    continue;
                };
                let price = state.price(good_id).unwrap_or(def.base_price);
                if price > def.base_price * MIL_INPUT_PRICE_FACTOR {
                    expensive_inputs.insert(good_idx);
                }
            }
        }
        // Also consider defs for mil types not yet in the world (e.g. first shipyard).
        for mil_id in mil_types {
            let Some(building_type) = self.defs.buildings.get(mil_id) else {
                continue;
            };
            for group_id in &building_type.production_method_groups {
                let Some(pms) = self.defs.production_method_groups.get(group_id) else {
                    continue;
                };
                for pm_id in pms {
                    let Some(pm) = self.defs.production_methods.get(pm_id) else {
                        continue;
                    };
                    for (good_idx, qty) in &pm.inputs {
                        if *qty <= ORDER_EPS {
                            continue;
                        }
                        let Some(good_id) = self.defs.good_by_index(*good_idx) else {
                            continue;
                        };
                        let Some(def) = self.defs.goods.get(good_id) else {
                            continue;
                        };
                        let price = state.price(good_id).unwrap_or(def.base_price);
                        if price > def.base_price * MIL_INPUT_PRICE_FACTOR {
                            expensive_inputs.insert(*good_idx);
                        }
                    }
                }
            }
        }
        for building in &world.buildings {
            if state
                .building_level_deltas
                .get(&building.building)
                .copied()
                .unwrap_or(0)
                >= u32::from(cap)
            {
                continue;
            }
            let (_, outputs) = building.goods_io(&self.defs);
            if expensive_inputs.iter().any(|idx| outputs[*idx] > ORDER_EPS) {
                candidates.insert(building.building.clone());
            }
        }
    }

    fn modeled_gdp(&self, state: &PlanningState, prices: &PricesResult) -> f64 {
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
        prices
            .buildings
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
        atoms: &[Atom],
        max_candidates: u16,
        max_overrides: u16,
    ) -> Vec<(u32, Vec<String>)> {
        if state.pm_overrides.len() >= usize::from(max_overrides) || max_candidates == 0 {
            return Vec::new();
        }
        let wants_price = atoms
            .iter()
            .any(|atom| matches!(atom, Atom::GoodPrice { .. }));
        let wants_gdp = atoms.iter().any(|atom| {
            matches!(
                atom,
                Atom::Gdp {
                    rel: Rel::Ge | Rel::Gt | Rel::Eq,
                    ..
                }
            )
        });
        if !wants_price && !wants_gdp {
            return Vec::new();
        }
        let world = self.world_for(state);
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
            let slot_count = building.production_methods.len().max(1);
            for slot in 0..slot_count {
                let current = building.production_methods.get(slot).cloned();
                let mut alternatives = BTreeSet::new();
                for group_id in &building_type.production_method_groups {
                    let Some(group) = self.defs.production_method_groups.get(group_id) else {
                        continue;
                    };
                    alternatives.extend(group.iter().cloned());
                }
                if alternatives.is_empty() {
                    alternatives.extend(building.production_methods.iter().cloned());
                    for other in &world.buildings {
                        if other.building == building.building {
                            alternatives.extend(other.production_methods.iter().cloned());
                        }
                    }
                }
                for candidate in alternatives {
                    if current.as_ref() == Some(&candidate) {
                        continue;
                    }
                    let relevant = pm_affects_open_atoms(&self.defs, &candidate, atoms, wants_gdp);
                    if !relevant {
                        continue;
                    }
                    let mut methods = if building.production_methods.is_empty() {
                        vec![candidate.clone()]
                    } else {
                        building.production_methods.clone()
                    };
                    if slot >= methods.len() {
                        methods.push(candidate);
                    } else {
                        methods[slot] = candidate;
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
        atoms: &[Atom],
        config: SimConfig,
    ) -> bool {
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

fn pm_affects_open_atoms(defs: &GameDefs, pm_id: &str, atoms: &[Atom], wants_gdp: bool) -> bool {
    let Some(pm) = defs.production_methods.get(pm_id) else {
        return wants_gdp;
    };
    if wants_gdp && (!pm.outputs.is_empty() || !pm.inputs.is_empty()) {
        return true;
    }
    for atom in atoms {
        let Atom::GoodPrice { good, rel, .. } = atom else {
            continue;
        };
        let Some(good_idx) = defs.index_of(good) else {
            continue;
        };
        let produces = pm
            .outputs
            .iter()
            .any(|(idx, qty)| *idx == good_idx && *qty > 0.0);
        let consumes = pm
            .inputs
            .iter()
            .any(|(idx, qty)| *idx == good_idx && *qty > 0.0);
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
    /// Fixed duration for one modeled building-level expansion.
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
    /// Max distinct building PM overrides on one planning branch.
    pub max_pm_overrides: u16,
    /// Max SwitchPm decision edges emitted in one expansion.
    pub max_pm_candidates: u16,
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
            max_pm_overrides: 4,
            max_pm_candidates: 4,
        }
    }
}

/// An event which can advance the simulation clock.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Event {
    /// The technology currently in the queue completes.
    TechCompleted { tech: String },
    /// One level of the queued building type completes.
    BuildingCompleted { building: String },
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
    /// Put one goal-relevant building level in the compact construction queue.
    QueueBuildingLevel { building: String },
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

/// One edge emitted by [`successors`] or [`successors_for_atoms`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Successor {
    pub action: Action,
    /// Edge cost in days. Decision edges have cost zero.
    pub days: u16,
    pub state: PlanningState,
}

/// Generate successors relevant to the currently unsatisfied atoms of `goal`.
pub fn successors(state: &PlanningState, goal: &Goal, config: SimConfig) -> Vec<Successor> {
    let open_atoms = gaps(goal, state);
    successors_for_atoms_with_economy(state, &open_atoms, config, None)
}

/// Generate successors with price-solver context for building decisions.
pub fn successors_with_economy(
    state: &PlanningState,
    goal: &Goal,
    config: SimConfig,
    economy: &EconomyContext,
) -> Vec<Successor> {
    let open_atoms = gaps(goal, state);
    successors_for_atoms_with_economy(state, &open_atoms, config, Some(economy))
}

/// Generate successors from an already-computed list of open goal atoms.
///
/// Decisions are gated per track (`research_busy`, `construction_busy`, …) so
/// research and construction may run in parallel. Instant `SwitchPm` /
/// `AdjustTax` decisions are not blocked by other tracks. At most one wait
/// edge is appended — the earliest completion among goal-relevant in-flight
/// tracks, or a payday tick when a solvency-related atom can move closer.
/// Idle states without an open solvency path emit no wait (I6).
pub fn successors_for_atoms(
    state: &PlanningState,
    open_atoms: &[Atom],
    config: SimConfig,
) -> Vec<Successor> {
    successors_for_atoms_with_economy(state, open_atoms, config, None)
}

fn interest_queued(kind: InterestKind, id: &str) -> QueuedInterest {
    match kind {
        InterestKind::State => QueuedInterest::State(id.to_string()),
        InterestKind::Region => QueuedInterest::Region(id.to_string()),
    }
}

fn interest_matches(queued: &QueuedInterest, kind: InterestKind, id: &str) -> bool {
    match (queued, kind) {
        (QueuedInterest::State(queued_id), InterestKind::State) => queued_id == id,
        (QueuedInterest::Region(queued_id), InterestKind::Region) => queued_id == id,
        _ => false,
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
    economy: Option<&EconomyContext>,
    config: SimConfig,
) {
    if let Some(next) = apply_action_with_economy(state, &action, economy, config) {
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
    economy: Option<&EconomyContext>,
    config: SimConfig,
) {
    // `days == 0` is allowed when the track timer has already elapsed.
    let action = Action::WaitForEvent { event, days };
    if let Some(next) = apply_action_with_economy(state, &action, economy, config) {
        result.push(Successor {
            action,
            days,
            state: next,
        });
    }
}

fn successors_for_atoms_with_economy(
    state: &PlanningState,
    open_atoms: &[Atom],
    config: SimConfig,
    economy: Option<&EconomyContext>,
) -> Vec<Successor> {
    let mut result = Vec::new();
    let mut seen_techs = BTreeSet::new();
    let mut seen_laws = BTreeSet::new();
    let mut seen_interest = BTreeSet::new();
    let mut seen_mil_builds = BTreeSet::new();
    let mut seen_mil_hires = BTreeSet::new();
    let mut seen_tax_deltas = BTreeSet::new();

    for atom in open_atoms {
        match atom {
            Atom::HasTech(tech) => {
                if state.research_busy() || state.has_tech(tech) || !seen_techs.insert(tech.clone())
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
            Atom::HasLaw(law) => {
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
            Atom::InterestIn { kind, id } => {
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
            Atom::ArmyPower { rel, value } => {
                push_military_pp_decisions(
                    &mut MilitaryPpDecisionArgs {
                        result: &mut result,
                        state,
                        economy,
                        config,
                        seen_builds: &mut seen_mil_builds,
                        seen_hires: &mut seen_mil_hires,
                    },
                    MilitaryBranch::Army,
                    *rel,
                    *value,
                );
            }
            Atom::NavyPower { rel, value } => {
                push_military_pp_decisions(
                    &mut MilitaryPpDecisionArgs {
                        result: &mut result,
                        state,
                        economy,
                        config,
                        seen_builds: &mut seen_mil_builds,
                        seen_hires: &mut seen_mil_hires,
                    },
                    MilitaryBranch::Navy,
                    *rel,
                    *value,
                );
            }
            Atom::WeeklyBalance { .. } => {
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

    if let Some(economy) = economy {
        for building in
            economy.building_candidates(state, open_atoms, config.max_added_levels_per_type)
        {
            push_decision(
                &mut result,
                state,
                Action::QueueBuildingLevel { building },
                Some(economy),
                config,
            );
        }
        for (building_id, methods) in economy.pm_switch_candidates(
            state,
            open_atoms,
            config.max_pm_candidates,
            config.max_pm_overrides,
        ) {
            push_decision(
                &mut result,
                state,
                Action::SwitchPm {
                    building_id,
                    methods,
                },
                Some(economy),
                config,
            );
        }
    }

    // Earliest goal-relevant wait among independent tracks.
    let mut wait_candidates: Vec<(u16, Event)> = Vec::new();
    if let Some(tech) = state.queued_tech.as_ref().filter(|queued| {
        open_atoms
            .iter()
            .any(|atom| atom.is_has_tech(queued.as_str()))
    }) {
        let days = state.tech_days_left.unwrap_or(config.research_days);
        wait_candidates.push((days, Event::TechCompleted { tech: tech.clone() }));
    }
    if let (Some(building), Some(_)) = (state.queued_building.as_ref(), economy) {
        if let Some(days) = construction_wait_days(state, config) {
            wait_candidates.push((
                days,
                Event::BuildingCompleted {
                    building: building.clone(),
                },
            ));
        }
    }
    if let Some(queued) = state.queued_interest.as_ref().filter(|queued| {
        open_atoms.iter().any(|atom| match atom {
            Atom::InterestIn { kind, id } => interest_matches(queued, *kind, id),
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
        open_atoms.iter().any(|atom| {
            matches!(atom, Atom::ArmyPower { .. } | Atom::NavyPower { .. }) && !atom.eval(state)
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
    if let Some(law) = state.queued_law.as_ref().filter(|queued| {
        open_atoms
            .iter()
            .any(|atom| atom.is_has_law(queued.as_str()))
    }) {
        let days = state.law_days_left.unwrap_or(config.law_days);
        wait_candidates.push((days, Event::LawEnacted { law: law.clone() }));
    }
    if payday_can_help(state, open_atoms) {
        wait_candidates.push((config.payday_days, Event::Payday {}));
    }

    if let Some((days, event)) = wait_candidates.into_iter().min_by_key(|(days, _)| *days) {
        push_wait(&mut result, state, event, days, economy, config);
    }

    result
}

/// Days until the construction head finishes under [`PlanningState::construction_rate`].
///
/// When `remaining` is present, uses [`crate::tracks::days_for_work`]. Otherwise
/// falls back to [`SimConfig::construction_days`]. Returns [`None`] when rate is
/// non-positive and remaining work is known (no finite ETA).
fn construction_wait_days(state: &PlanningState, config: SimConfig) -> Option<u16> {
    let remaining = state
        .constructions
        .first()
        .and_then(|row| row.remaining)
        .filter(|v| v.is_finite() && *v >= 0.0);
    match remaining {
        Some(work) => {
            let days = crate::tracks::days_for_work(work, state.construction_rate)?;
            Some(u16::try_from(days).unwrap_or(u16::MAX))
        }
        None => Some(config.construction_days),
    }
}

fn tax_deltas_toward(state: &PlanningState, atom: &Atom, config: SimConfig) -> Vec<i8> {
    let Atom::WeeklyBalance { rel, value } = atom else {
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

fn is_solvency_atom(atom: &Atom) -> bool {
    matches!(
        atom,
        Atom::Solvent | Atom::CreditHeadroom { .. } | Atom::DebtPrincipal { .. }
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

fn fiscal_slack(atom: &Atom, state: &PlanningState) -> Option<f64> {
    match atom {
        Atom::Solvent => Some(if state.solvent { 0.0 } else { 1.0 }),
        Atom::CreditHeadroom { rel, value } => {
            let headroom = state.credit_headroom?;
            if rel.holds(headroom, *value) {
                Some(0.0)
            } else {
                Some((headroom - *value).abs())
            }
        }
        Atom::DebtPrincipal { rel, value } => {
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
fn payday_can_help(state: &PlanningState, open_atoms: &[Atom]) -> bool {
    if !open_atoms.iter().any(is_solvency_atom) {
        return false;
    }
    let mut next = state.clone();
    apply_payday_effects(&mut next);
    if next == *state {
        return false;
    }
    open_atoms.iter().any(|atom| {
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
/// Applying the same `(state, action, config)` always yields the same result (I8).
pub fn apply_action(
    state: &PlanningState,
    action: &Action,
    config: SimConfig,
) -> Option<PlanningState> {
    apply_action_with_economy(state, action, None, config)
}

/// Apply an action with optional price-solver context and sim timing config.
pub fn apply_action_with_economy(
    state: &PlanningState,
    action: &Action,
    economy: Option<&EconomyContext>,
    config: SimConfig,
) -> Option<PlanningState> {
    let mut next = state.clone();
    match action {
        Action::QueueTech { tech } => {
            if tech.is_empty() || next.research_busy() || next.has_tech(tech) {
                return None;
            }
            next.queued_tech = Some(tech.clone());
            next.tech_days_left = Some(config.research_days);
        }
        Action::QueueBuildingLevel { building } => {
            if building.is_empty() || next.construction_busy() || economy.is_none() {
                return None;
            }
            // Synthetic work so parallel ticks and ETA share one representation:
            // `ceil(remaining / rate) == construction_days` at enqueue time.
            let remaining = f64::from(config.construction_days) * next.construction_rate;
            next.push_construction(building.clone());
            if let Some(entry) = next.constructions.last_mut() {
                entry.remaining = Some(remaining);
            }
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
            let economy = economy?;
            if methods.is_empty() {
                return None;
            }
            if next.pm_overrides.get(building_id) == Some(methods) {
                return None;
            }
            let world = economy.world_for(&next);
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

/// Seed missing timers from `config` so parallel ticks advance save-loaded queues.
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
    if let Some(head) = state.constructions.first_mut() {
        if head.remaining.is_none() {
            head.remaining = Some(f64::from(config.construction_days) * state.construction_rate);
        }
    }
}

fn construction_timer_elapsed(state: &PlanningState) -> bool {
    state
        .constructions
        .first()
        .and_then(|row| row.remaining)
        .is_some_and(|rem| rem.is_finite() && rem <= 0.0)
}

fn apply_wait_for_event(
    next: &mut PlanningState,
    event: &Event,
    days: u16,
    economy: Option<&EconomyContext>,
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
            next.tick_parallel_tracks(days);
            next.date = next.date.add_days(i32::from(days));
            next.queued_tech = None;
            next.tech_days_left = None;
            next.techs.insert(tech.clone());
        }
        Event::BuildingCompleted { building } => {
            let economy = economy?;
            if next.queued_building.as_deref() != Some(building.as_str()) {
                return None;
            }
            if days == 0 && !construction_timer_elapsed(next) {
                return None;
            }
            next.tick_parallel_tracks(days);
            next.date = next.date.add_days(i32::from(days));
            next.complete_construction(building);
            *next
                .building_level_deltas
                .entry(building.clone())
                .or_default() += 1;
            if is_military_planning_building(building) {
                next.push_mil_building_level(building);
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
            next.tick_parallel_tracks(days);
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
            next.tick_parallel_tracks(days);
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
            next.tick_parallel_tracks(days);
            next.date = next.date.add_days(i32::from(days));
            next.queued_law = None;
            next.law_days_left = None;
            next.laws.insert(law.clone());
        }
        Event::Payday {} => {
            if days == 0 {
                return None;
            }
            next.tick_parallel_tracks(days);
            next.date = next.date.add_days(i32::from(days));
            apply_payday_effects(next);
        }
    }
    Some(())
}

fn refresh_prices(state: &mut PlanningState, economy: &EconomyContext) {
    let prices = solve(
        &economy.world_for(state),
        &economy.defs,
        economy.solve_opts.clone(),
    );
    state.gdp = economy.modeled_gdp(state, &prices);
    state.good_prices = prices
        .goods
        .into_iter()
        .map(|good| (good.id, good.price))
        .collect();
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::{compile, evaluate};
    use crate::world::{PlanningParts, Vic3Date};
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use vic3_defs::{Good, GoodIdx, GoodsVec};
    use vic3_prices::{WorldBuilding, WorldCountry, WorldState};

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

        let decisions = successors(&start, &goal, config);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].days, 0);
        assert!(matches!(
            decisions[0].action,
            Action::QueueInterest {
                kind: InterestKind::State,
                ref id
            } if id == "alsace"
        ));

        let waits = successors(&decisions[0].state, &goal, config);
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
            let edges = successors(&state, &goal, config);
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

        let decisions = successors(&start, &goal, config);
        assert_eq!(decisions.len(), 1);
        assert!(matches!(
            decisions[0].action,
            Action::QueueHireMilitary { ref building } if building == BUILDING_BARRACKS
        ));

        let waits = successors(&decisions[0].state, &goal, config);
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
        let decisions = successors(&start, &goal, SimConfig::default());
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
        );
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 100);
        assert_eq!(start.date.days_until(&waits[0].state.date), 100);
        assert!(waits[0].state.queued_tech.is_none());
        assert!(evaluate(&goal, &waits[0].state));
    }

    #[test]
    fn building_level_then_wait_reaches_good_price() {
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
                saved_outputs: vec![(GoodIdx::from_usize(0), 10.0)],
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
        let goal = Goal::Atom(Atom::GoodPrice {
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
            max_added_levels_per_type: 2,
            ..SimConfig::default()
        };

        let gdp_goal = Goal::Atom(Atom::Gdp {
            rel: Rel::Ge,
            value: (initial_gdp + next_gdp) / 2.0,
        });
        let gdp_decisions = successors_with_economy(&state, &gdp_goal, config, &economy);
        assert!(matches!(
            gdp_decisions.as_slice(),
            [Successor {
                action: Action::QueueBuildingLevel { building },
                ..
            }] if building == "building_logging_camp"
        ));

        let decisions = successors_with_economy(&state, &goal, config, &economy);
        assert!(matches!(
            decisions.as_slice(),
            [Successor {
                action: Action::QueueBuildingLevel { building },
                days: 0,
                ..
            }] if building == "building_logging_camp"
        ));
        let repeated_decision =
            apply_action_with_economy(&state, &decisions[0].action, Some(&economy), config)
                .unwrap();
        assert_eq!(repeated_decision, decisions[0].state);
        assert_eq!(
            repeated_decision.fingerprint(),
            decisions[0].state.fingerprint()
        );
        let waits = successors_with_economy(&decisions[0].state, &goal, config, &economy);
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 30);
        assert!(matches!(
            waits[0].action,
            Action::WaitForEvent {
                event: Event::BuildingCompleted { .. },
                days: 30,
            }
        ));
        assert!(evaluate(&goal, &waits[0].state));
        assert_eq!(waits[0].state.gdp, next_gdp);
        assert!(evaluate(&gdp_goal, &waits[0].state));
        let repeated_wait = apply_action_with_economy(
            &decisions[0].state,
            &waits[0].action,
            Some(&economy),
            config,
        )
        .unwrap();
        assert_eq!(repeated_wait, waits[0].state);
        assert_eq!(repeated_wait.fingerprint(), waits[0].state.fingerprint());
        assert_eq!(
            waits[0]
                .state
                .building_level_deltas
                .get("building_logging_camp"),
            Some(&1)
        );

        let unreachable_gdp = Goal::Atom(Atom::Gdp {
            rel: Rel::Ge,
            value: f64::MAX,
        });
        let mut capped = state;
        for _ in 0..config.max_added_levels_per_type {
            let queue = successors_with_economy(&capped, &unreachable_gdp, config, &economy);
            assert_eq!(queue.len(), 1);
            let complete =
                successors_with_economy(&queue[0].state, &unreachable_gdp, config, &economy);
            assert_eq!(complete.len(), 1);
            capped = complete[0].state.clone();
        }
        assert!(
            successors_with_economy(&capped, &unreachable_gdp, config, &economy).is_empty(),
            "per-type cap must make an unreachable GDP search finite"
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
        let waits = successors(&start, &goal, config);
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

        let repeated = apply_action(&start, &waits[0].action, config).unwrap();
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
            let waits = successors(&state, &goal, config);
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
        assert!(successors(&deficit, &compile("solvent").unwrap(), config).is_empty());

        let wealth_only = successors_for_atoms(
            &insolvent,
            &[Atom::PopulationWeightedWealth {
                rel: Rel::Ge,
                value: 20.0,
            }],
            config,
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

        let decisions = successors(&start, &goal, config);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].days, 0);
        assert!(matches!(
            decisions[0].action,
            Action::QueueLaw { ref law } if law == "law_homesteading"
        ));

        let waits = successors(&decisions[0].state, &goal, config);
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

        let decisions = successors(&start, &goal, config);
        assert_eq!(decisions.len(), 1);
        assert!(matches!(
            decisions[0].action,
            Action::AdjustTax { delta: 1, .. }
        ));
        assert_eq!(decisions[0].state.weekly_balance, Some(90.0));
        assert!(!evaluate(&goal, &decisions[0].state));

        let second = successors(&decisions[0].state, &goal, config);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].state.weekly_balance, Some(140.0));
        assert!(evaluate(&goal, &second[0].state));
    }

    #[test]
    fn switch_pm_then_resolves_good_price() {
        use vic3_defs::{BuildingType, ProductionMethod};

        let wood = GoodIdx::from_usize(0);
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
        let goal = Goal::Atom(Atom::GoodPrice {
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
            ..SimConfig::default()
        };

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
                saved_outputs: vec![(GoodIdx::from_usize(0), 10.0)],
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
            construction_rate: 10.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let goal = Goal::Atom(Atom::GoodPrice {
            good: "wood".into(),
            rel: Rel::Le,
            value: 0.0,
        });
        let waits = successors_with_economy(&state, &goal, SimConfig::default(), &economy);
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
                saved_outputs: vec![(GoodIdx::from_usize(0), 10.0)],
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
            construction_rate: 1.0,
            good_prices: vec![("wood".into(), 30.0)],
            ..PlanningParts::default()
        });
        let start_date = state.date;
        let goal = Goal::And(vec![
            Goal::Atom(Atom::HasTech("nitroglycerin".into())),
            Goal::Atom(Atom::GoodPrice {
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
    fn mil_queue_building_ok_while_research_busy() {
        let defs = GameDefs::default();
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
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
                    Action::QueueBuildingLevel { building } if building == BUILDING_BARRACKS
                )
            }),
            "military construction must not be blocked by research: {edges:?}"
        );
        assert!(apply_action_with_economy(
            &start,
            &Action::QueueBuildingLevel {
                building: BUILDING_BARRACKS.into(),
            },
            Some(&economy),
            SimConfig::default(),
        )
        .is_some());
        assert!(
            apply_action(
                &start,
                &Action::QueueTech {
                    tech: "nitroglycerin".into(),
                },
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
        let next = apply_action(&start, &wait, SimConfig::default()).unwrap();
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
        assert!(apply_action(&not_ready, &rejected, SimConfig::default()).is_none());
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
        let a = apply_action(&state, &action, config).unwrap();
        let b = apply_action(&state.clone(), &action.clone(), config).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());

        let wait = Action::WaitForEvent {
            event: Event::TechCompleted {
                tech: "railways".into(),
            },
            days: 45,
        };
        let a = apply_action(&a, &wait, config).unwrap();
        let b = apply_action(&b, &wait, config).unwrap();
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
            let edges = successors(&state, &goal, config);

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

            let idle_atoms = [Atom::Solvent];
            let idle_edges = successors_for_atoms(
                &state_at(day_offset),
                &idle_atoms,
                config,
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
