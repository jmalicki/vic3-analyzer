//! Goal-relevant simulator successors over [`PlanningState`].
//!
//! Decision edges cost zero days. An expansion contains at most one event-wait
//! edge, selected from events already in flight. This crate deliberately does
//! not perform graph search.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use vic3_defs::GameDefs;
use vic3_goals::{gaps, Atom, Goal, Rel};
use vic3_prices::{solve, PricesResult, SolveOpts, World, ORDER_EPS};
use vic3_world::PlanningState;

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
        state
            .building_level_deltas
            .iter()
            .fold(self.base_world.clone(), |world, (building, levels)| {
                world.with_extra_levels(building, *levels)
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
        candidates.into_iter().collect()
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
}

/// Tunable durations used by the compact simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimConfig {
    /// Fixed research duration in the phase-8 model.
    pub research_days: u16,
    /// Fixed duration for one modeled building-level expansion.
    pub construction_days: u16,
    /// Finite search bound for added levels of one building type.
    pub max_added_levels_per_type: u16,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            research_days: 365,
            construction_days: 180,
            max_added_levels_per_type: 10,
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
}

/// A deterministic state transition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// Put a goal-relevant technology in the empty research queue.
    QueueTech { tech: String },
    /// Put one goal-relevant building level in the compact construction queue.
    QueueBuildingLevel { building: String },
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
/// Technology decisions are emitted in atom order with duplicates removed.
/// At most one wait edge is appended. A queued technology is the only in-flight
/// event represented by the compact phase-8 state; therefore an idle state has
/// no wait edge. Payday is intentionally not invented when no solvency model is
/// present.
pub fn successors_for_atoms(
    state: &PlanningState,
    open_atoms: &[Atom],
    config: SimConfig,
) -> Vec<Successor> {
    successors_for_atoms_with_economy(state, open_atoms, config, None)
}

fn successors_for_atoms_with_economy(
    state: &PlanningState,
    open_atoms: &[Atom],
    config: SimConfig,
    economy: Option<&EconomyContext>,
) -> Vec<Successor> {
    let mut result = Vec::new();
    let mut seen_techs = std::collections::BTreeSet::new();

    if state.queued_tech.is_none() && state.queued_building.is_none() {
        for atom in open_atoms {
            let Atom::HasTech(tech) = atom else {
                continue;
            };
            if state.has_tech(tech) || !seen_techs.insert(tech.clone()) {
                continue;
            }
            let action = Action::QueueTech { tech: tech.clone() };
            if let Some(next) = apply_action_with_economy(state, &action, economy) {
                result.push(Successor {
                    action,
                    days: 0,
                    state: next,
                });
            }
        }

        if let Some(economy) = economy {
            for building in
                economy.building_candidates(state, open_atoms, config.max_added_levels_per_type)
            {
                let action = Action::QueueBuildingLevel { building };
                if let Some(next) = apply_action_with_economy(state, &action, Some(economy)) {
                    result.push(Successor {
                        action,
                        days: 0,
                        state: next,
                    });
                }
            }
        }
    }

    if let Some(tech) = state.queued_tech.as_ref().filter(|queued| {
        open_atoms
            .iter()
            .any(|atom| atom.is_has_tech(queued.as_str()))
    }) {
        let days = config.research_days.max(1);
        let action = Action::WaitForEvent {
            event: Event::TechCompleted { tech: tech.clone() },
            days,
        };
        if let Some(next) = apply_action_with_economy(state, &action, economy) {
            result.push(Successor {
                action,
                days,
                state: next,
            });
        }
    } else if let (Some(building), Some(economy)) = (state.queued_building.as_ref(), economy) {
        let days = config.construction_days.max(1);
        let action = Action::WaitForEvent {
            event: Event::BuildingCompleted {
                building: building.clone(),
            },
            days,
        };
        if let Some(next) = apply_action_with_economy(state, &action, Some(economy)) {
            result.push(Successor {
                action,
                days,
                state: next,
            });
        }
    }

    result
}

/// Apply an action if its preconditions hold.
///
/// The action carries all timing information, so applying it to identical
/// states always produces identical states (I8).
pub fn apply_action(state: &PlanningState, action: &Action) -> Option<PlanningState> {
    apply_action_with_economy(state, action, None)
}

/// Apply an action with optional price-solver context.
pub fn apply_action_with_economy(
    state: &PlanningState,
    action: &Action,
    economy: Option<&EconomyContext>,
) -> Option<PlanningState> {
    let mut next = state.clone();
    match action {
        Action::QueueTech { tech } => {
            if tech.is_empty()
                || next.queued_tech.is_some()
                || next.queued_building.is_some()
                || next.has_tech(tech)
            {
                return None;
            }
            next.queued_tech = Some(tech.clone());
        }
        Action::QueueBuildingLevel { building } => {
            if building.is_empty()
                || next.queued_tech.is_some()
                || next.queued_building.is_some()
                || economy.is_none()
            {
                return None;
            }
            next.queued_building = Some(building.clone());
        }
        Action::WaitForEvent {
            event: Event::TechCompleted { tech },
            days,
        } => {
            if *days == 0 || next.queued_tech.as_deref() != Some(tech.as_str()) {
                return None;
            }
            next.date = next.date.add_days(i32::from(*days));
            next.queued_tech = None;
            next.techs.insert(tech.clone());
        }
        Action::WaitForEvent {
            event: Event::BuildingCompleted { building },
            days,
        } => {
            let economy = economy?;
            if *days == 0 || next.queued_building.as_deref() != Some(building.as_str()) {
                return None;
            }
            next.date = next.date.add_days(i32::from(*days));
            next.queued_building = None;
            *next
                .building_level_deltas
                .entry(building.clone())
                .or_default() += 1;
            let prices = solve(
                &economy.world_for(&next),
                &economy.defs,
                economy.solve_opts.clone(),
            );
            next.gdp = economy.modeled_gdp(&next, &prices);
            next.good_prices = prices
                .goods
                .into_iter()
                .map(|good| (good.id, good.price))
                .collect();
        }
    }
    Some(next)
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use vic3_defs::{Good, GoodIdx, GoodsVec};
    use vic3_goals::{compile, evaluate};
    use vic3_prices::{WorldBuilding, WorldCountry, WorldState};
    use vic3_world::{PlanningParts, Vic3Date};

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
        let baseline = solve(&world, &defs, solve_opts);
        let bumped = solve(
            &world.with_extra_levels("building_logging_camp", 1),
            &defs,
            solve_opts,
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
            apply_action_with_economy(&state, &decisions[0].action, Some(&economy)).unwrap();
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
        let repeated_wait =
            apply_action_with_economy(&decisions[0].state, &waits[0].action, Some(&economy))
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
    fn i8_actions_deterministic_and_hash_stable() {
        let state = state_at(7);
        let action = Action::QueueTech {
            tech: "railways".into(),
        };
        let a = apply_action(&state, &action).unwrap();
        let b = apply_action(&state.clone(), &action.clone()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());

        let wait = Action::WaitForEvent {
            event: Event::TechCompleted {
                tech: "railways".into(),
            },
            days: 45,
        };
        let a = apply_action(&a, &wait).unwrap();
        let b = apply_action(&b, &wait).unwrap();
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
                if matches!(&edge.action, Action::WaitForEvent { .. }) {
                    prop_assert!(edge.state.date > state.date);
                }
            }

            if !queue_tech {
                prop_assert_eq!(wait_count, 0);
            }

            let idle_atoms = [Atom::ArmyPower {
                rel: vic3_goals::Rel::Ge,
                value: 100.0,
            }];
            let idle_edges = successors_for_atoms(
                &state_at(day_offset),
                &idle_atoms,
                config,
            );
            let has_wait = idle_edges
                .iter()
                .any(|edge| matches!(&edge.action, Action::WaitForEvent { .. }));
            prop_assert!(!has_wait);
        }
    }
}
