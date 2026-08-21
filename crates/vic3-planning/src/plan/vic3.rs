//! A* node backed by goal-relevant [`crate::sim`] transitions.
//!
//! # Cheap intern key
//!
//! [`Vic3Node`] Eq/Hash inspect only the precomputed `u64` fingerprint of
//! [`PlanningState`]. The projected state and immutable search inputs
//! (goal, [`SimConfig`], optional [`EconomyContext`]) ride behind [`Arc`]s and
//! are never deeply compared by the pathfinder.
//!
//! # Heuristic DAG
//!
//! [`SearchNode::heuristic`] walks the compiled [`Goal`] as a dependency DAG:
//! AND → max child, OR → min child, NOT → 0. Open research / interest /
//! raisable army / law atoms contribute fixed model days even if an unrelated
//! item is queued (returning 0 over a zero-day queue edge would break
//! consistency). Open `good_price` / `gdp` use `construction_days` unless a
//! zero-day SwitchPm path exists. Fiscal / SoL / tax atoms contribute 0.
//!
//! Admissible relaxation of the real graph (**I7** on research formulas), not
//! a substitute for search.

use super::pathfinding::SearchNode;
use crate::goals::{evaluate, Atom, Goal};
use crate::sim::{EconomyContext, SimConfig, Successor};
use crate::world::PlanningState;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Immutable inputs shared by every node in one planning search.
#[derive(Debug)]
struct SearchContext {
    goal: Goal,
    config: SimConfig,
    economy: Option<Arc<EconomyContext>>,
}

/// Compact key and state handle for Vic3 planning.
///
/// Nodes created by one search all share the same goal and simulator config.
/// The `fingerprint` is the complete intern-map identity; `state` and `context`
/// remain available for simulation without becoming fat hash-map key bodies.
#[derive(Clone, Debug)]
pub struct Vic3Node {
    fingerprint: u64,
    state: Arc<PlanningState>,
    context: Arc<SearchContext>,
}

impl Vic3Node {
    /// Create the root of a planning search.
    pub fn new(state: PlanningState, goal: Goal, config: SimConfig) -> Self {
        Self::with_context(
            state,
            Arc::new(SearchContext {
                goal,
                config,
                economy: None,
            }),
        )
    }

    /// Create a root with immutable price-solver context for building actions.
    pub fn new_with_economy(
        state: PlanningState,
        goal: Goal,
        config: SimConfig,
        economy: EconomyContext,
    ) -> Self {
        Self::with_context(
            state,
            Arc::new(SearchContext {
                goal,
                config,
                economy: Some(Arc::new(economy)),
            }),
        )
    }

    fn with_context(state: PlanningState, context: Arc<SearchContext>) -> Self {
        Self {
            fingerprint: state.fingerprint(),
            state: Arc::new(state),
            context,
        }
    }

    /// Compact identity used by the pathfinder's intern map.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Projected world state represented by this node.
    pub fn state(&self) -> &PlanningState {
        &self.state
    }

    /// Compiled goal shared by this search.
    pub fn goal(&self) -> &Goal {
        &self.context.goal
    }

    /// Simulator timing configuration shared by this search.
    pub fn config(&self) -> SimConfig {
        self.context.config
    }

    pub(crate) fn sim_successors(&self) -> Vec<Successor> {
        match self.context.economy.as_deref() {
            Some(economy) => crate::sim::successors_with_economy(
                &self.state,
                &self.context.goal,
                self.context.config,
                economy,
            ),
            None => crate::sim::successors(&self.state, &self.context.goal, self.context.config),
        }
    }
}

impl PartialEq for Vic3Node {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl Eq for Vic3Node {}

impl Hash for Vic3Node {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

/// Admissible remaining-days bound from the compiled goal's dependency DAG.
///
/// Atomic timing is known for research, interest declarations, army power
/// expansions, law enactments, and (when no zero-day PM path exists)
/// construction. AND uses the longest child (actions may overlap or satisfy
/// multiple atoms), OR uses the cheapest child, and NOT stays at zero. Missing
/// technologies / open interest / raisable army / missing laws always
/// contribute their fixed duration, even when an unrelated item is queued:
/// returning zero there would drop the estimate over a zero-day queue edge and
/// break consistency for closed-node A*. Open `good_price` / `gdp` atoms
/// contribute `construction_days` when the economy context has no SwitchPm
/// candidate; otherwise they contribute zero because PM switches are 0-day.
/// Fiscal, SoL, tax, and other atoms without a proven timing model contribute
/// zero. This is deliberately a relaxation of the real state graph, not a
/// replacement for A*.
fn goal_timing_lower_bound(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: Option<&EconomyContext>,
) -> u32 {
    let research_days = u32::from(config.research_days.max(1));
    let interest_days = u32::from(config.interest_days.max(1));
    let army_train_days = u32::from(config.army_training_days.max(1));
    let navy_crew_days = u32::from(config.navy_crew_days.max(1));
    let law_days = u32::from(config.law_days.max(1));
    let construction_days = u32::from(config.construction_days.max(1));
    match goal {
        Goal::And(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config, economy))
            .max()
            .unwrap_or(0),
        Goal::Or(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config, economy))
            .min()
            .unwrap_or(0),
        Goal::Not(_) => 0,
        Goal::Atom(Atom::HasTech(tech)) => {
            if state.has_tech(tech) {
                0
            } else {
                research_days
            }
        }
        Goal::Atom(Atom::HasLaw(law)) => {
            if state.has_law(law) {
                0
            } else {
                law_days
            }
        }
        Goal::Atom(Atom::InterestIn { kind, id }) => {
            let held = match kind {
                crate::goals::InterestKind::State => state.has_interest_state(id),
                crate::goals::InterestKind::Region => state.has_interest_region(id),
            };
            if held {
                0
            } else {
                interest_days
            }
        }
        Goal::Atom(atom @ Atom::ArmyPower { rel, value }) => {
            if atom.eval(state) {
                0
            } else if !state.army_buildings_fully_staffed() {
                army_train_days
            } else if crate::sim::power_raise_needed(*rel, *value, state.army_power_projection)
                .is_some()
            {
                construction_days.saturating_add(army_train_days)
            } else {
                0
            }
        }
        Goal::Atom(atom @ Atom::NavyPower { rel, value }) => {
            if atom.eval(state) {
                0
            } else if !state.navy_buildings_fully_staffed() {
                navy_crew_days
            } else if crate::sim::power_raise_needed(*rel, *value, state.navy_power_projection)
                .is_some()
            {
                construction_days.saturating_add(navy_crew_days)
            } else {
                0
            }
        }
        Goal::Atom(atom @ (Atom::GoodPrice { .. } | Atom::Gdp { .. })) => {
            if atom.eval(state)
                || economy.is_some_and(|economy| {
                    economy.has_pm_switch_path(state, std::slice::from_ref(atom), config)
                })
            {
                0
            } else {
                construction_days
            }
        }
        Goal::Atom(_) => 0,
    }
}

impl SearchNode for Vic3Node {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, Self::Cost)> {
        self.sim_successors()
            .into_iter()
            .map(|successor| {
                (
                    Self::with_context(successor.state, Arc::clone(&self.context)),
                    u32::from(successor.days),
                )
            })
            .collect()
    }

    fn is_goal(&self) -> bool {
        evaluate(&self.context.goal, &self.state)
    }

    /// Goal-DAG relaxation: exact for research / interest / raisable army / law
    /// atoms, construction days for open price/GDP when no SwitchPm path exists,
    /// zero for atoms without a proven timing model.
    fn heuristic(&self) -> Self::Cost {
        goal_timing_lower_bound(
            &self.context.goal,
            &self.state,
            self.context.config,
            self.context.economy.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::plan::pathfinding::{shortest_path, shortest_path_lazy};
    use crate::sim::Action;
    use crate::world::{PlanningParts, PlanningState};
    use proptest::prelude::*;
    use rust_advanced_heaps::pairing::PairingHeap;
    use rust_advanced_heaps::simple_binary::SimpleBinaryHeap;

    fn tech_fixture(research_days: u16) -> Vic3Node {
        Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                ..PlanningParts::default()
            }),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig {
                research_days,
                ..SimConfig::default()
            },
        )
    }

    #[test]
    fn interest_and_army_plan_closes_declare_war_when_munitions_solvent_ok() {
        // Army PP already meets the declare-war threshold; only interest remains.
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                solvent: true,
                good_prices: vec![("ammunition".into(), 30.0)],
                army_power_projection: Some(150.0),
                army_pp_baseline: Some(150.0),
                ..PlanningParts::default()
            }),
            compile("declare-war(state=alsace)").unwrap(),
            SimConfig {
                interest_days: 30,
                ..SimConfig::default()
            },
        );

        assert_eq!(
            start.heuristic(),
            30,
            "AND bound is interest when army holds"
        );
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("declare-war reachable");
        assert_eq!(cost, 30);
        let goal_node = path.last().unwrap();
        assert!(goal_node.is_goal());
        assert!(goal_node.state().has_interest_state("alsace"));
        assert!(goal_node
            .state()
            .army_power_projection
            .is_some_and(|p| p >= 100.0));
    }

    #[test]
    fn army_only_plan_hires_staffed_barracks() {
        use crate::military::{ModeledMilBuilding, UnitCombatStats, BUILDING_BARRACKS};
        let per = UnitCombatStats::army_default().full_power_projection();
        let levels = (100.0 / per).ceil();
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                army_power_projection: Some(0.0),
                army_pp_baseline: Some(0.0),
                mil_buildings: vec![ModeledMilBuilding {
                    building: BUILDING_BARRACKS.into(),
                    levels,
                    staffing: 0.0,
                }],
                ..PlanningParts::default()
            }),
            compile("army_power_projection >= 100").unwrap(),
            SimConfig {
                army_training_days: 77,
                ..SimConfig::default()
            },
        );
        assert_eq!(start.heuristic(), 77);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("army goal reachable");
        assert_eq!(cost, 77);
        assert!(path.last().unwrap().is_goal());
        assert!(path
            .last()
            .unwrap()
            .state()
            .army_power_projection
            .is_some_and(|p| p >= 100.0));
    }

    #[test]
    fn interest_only_plan_cost_is_queue_plus_wait_days() {
        let start = Vic3Node::new(
            PlanningState::default(),
            compile("interest_in(region=region_western_europe)").unwrap(),
            SimConfig {
                interest_days: 25,
                ..SimConfig::default()
            },
        );
        assert_eq!(start.heuristic(), 25);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("interest goal reachable");
        assert_eq!(cost, 25);
        assert!(path[2].state().has_interest_region("region_western_europe"));
        assert!(path[2].is_goal());
    }

    #[test]
    fn tech_only_plan_cost_is_queue_plus_wait_days() {
        let start = tech_fixture(100);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("tech goal is reachable");

        assert_eq!(cost, 100);
        assert_eq!(path.len(), 3);
        assert!(path[0].state().queued_tech.is_none());
        assert_eq!(
            path[1].state().queued_tech.as_deref(),
            Some("nitroglycerin")
        );
        assert!(path[2].state().has_tech("nitroglycerin"));
        assert!(path[2].is_goal());

        let first_edges = crate::sim::successors(start.state(), start.goal(), start.config());
        assert!(matches!(
            first_edges.as_slice(),
            [crate::sim::Successor {
                action: Action::QueueTech { tech },
                days: 0,
                ..
            }] if tech == "nitroglycerin"
        ));
    }

    #[test]
    fn payday_plan_closes_solvent() {
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                debt_principal: Some(1_000.0),
                credit_limit: Some(1_000.0),
                credit_headroom: Some(0.0),
                solvent: false,
                weekly_balance: Some(100.0),
                treasury: 0.0,
                ..PlanningParts::default()
            }),
            compile("solvent").unwrap(),
            SimConfig {
                payday_days: 7,
                ..SimConfig::default()
            },
        );
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("solvent is reachable");
        assert_eq!(cost, 7);
        assert!(path.last().is_some_and(|node| node.is_goal()));
        assert!(path.last().unwrap().state().solvent);
        assert_eq!(path.last().unwrap().state().credit_headroom, Some(100.0));
    }

    #[test]
    fn law_plan_cost_is_queue_plus_wait_days() {
        let start = Vic3Node::new(
            PlanningState::default(),
            compile("has_law(law_homesteading)").unwrap(),
            SimConfig {
                law_days: 55,
                ..SimConfig::default()
            },
        );
        assert_eq!(start.heuristic(), 55);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("law goal reachable");
        assert_eq!(cost, 55);
        assert!(path.last().unwrap().state().has_law("homesteading"));
        assert!(path.last().unwrap().is_goal());
    }

    #[test]
    fn tax_plan_closes_weekly_balance_in_zero_days() {
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                weekly_balance: Some(20.0),
                ..PlanningParts::default()
            }),
            compile("weekly_balance >= 100").unwrap(),
            SimConfig {
                tax_balance_per_step: 50,
                max_tax_steps: 3,
                ..SimConfig::default()
            },
        );
        assert_eq!(start.heuristic(), 0);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("tax goal reachable");
        assert_eq!(cost, 0);
        assert!(path.last().unwrap().is_goal());
        assert!(path.last().unwrap().state().weekly_balance.unwrap() >= 100.0);
    }

    #[test]
    fn construction_bound_applies_without_pm_path() {
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                good_prices: vec![("wood".into(), 40.0)],
                ..PlanningParts::default()
            }),
            compile("good_price(wood) <= 20").unwrap(),
            SimConfig {
                construction_days: 33,
                ..SimConfig::default()
            },
        );
        assert_eq!(
            start.heuristic(),
            33,
            "without economy PM candidates, open price uses construction days"
        );
    }

    #[test]
    fn shortest_path_agrees_with_lazy_on_tech_fixture() {
        let start = tech_fixture(37);
        let pairing = shortest_path::<_, PairingHeap<_, _>>(&start).map(|(_, cost)| cost);
        let lazy = shortest_path_lazy::<_, SimpleBinaryHeap<_, _>>(&start).map(|(_, cost)| cost);
        assert_eq!(pairing, lazy);
        assert_eq!(pairing, Some(37));
    }

    #[test]
    fn goal_dag_bound_handles_research_and_or_dependencies() {
        let one = tech_fixture(40);
        assert_eq!(one.heuristic(), 40);

        let and = Vic3Node::new(
            PlanningState::default(),
            compile("has_tech(railways) && has_tech(nitroglycerin)").unwrap(),
            SimConfig {
                research_days: 40,
                ..SimConfig::default()
            },
        );
        let (_, and_cost) =
            shortest_path::<_, PairingHeap<_, _>>(&and).expect("two techs are reachable");
        assert_eq!(and.heuristic(), 40);
        assert_eq!(and_cost, 80);

        let or = Vic3Node::new(
            PlanningState::default(),
            compile("has_tech(railways) || has_tech(nitroglycerin)").unwrap(),
            SimConfig {
                research_days: 40,
                ..SimConfig::default()
            },
        );
        let (_, or_cost) =
            shortest_path::<_, PairingHeap<_, _>>(&or).expect("either tech is reachable");
        assert_eq!(or.heuristic(), 40);
        assert_eq!(or_cost, 40);
    }

    #[test]
    fn missing_tech_bound_ignores_queue_identity() {
        let config = SimConfig {
            research_days: 40,
            ..SimConfig::default()
        };
        let goal = compile("research(tech=railways)").unwrap();

        let idle = Vic3Node::new(PlanningState::default(), goal.clone(), config);
        assert_eq!(idle.heuristic(), 40);

        let matching = PlanningState {
            queued_tech: Some("railways".into()),
            ..PlanningState::default()
        };
        assert_eq!(
            Vic3Node::new(matching, goal.clone(), config).heuristic(),
            40
        );

        let unrelated = PlanningState {
            queued_tech: Some("unrelated_tech".into()),
            ..PlanningState::default()
        };
        assert_eq!(Vic3Node::new(unrelated, goal, config).heuristic(), 40);
    }

    #[test]
    fn zero_day_queue_edge_preserves_heuristic() {
        let start = Vic3Node::new(
            PlanningState::default(),
            compile("has_tech(railways) || has_tech(nitroglycerin)").unwrap(),
            SimConfig {
                research_days: 40,
                ..SimConfig::default()
            },
        );
        assert_eq!(start.heuristic(), 40);
        for (successor, days) in start.successors() {
            assert_eq!(days, 0);
            assert_eq!(
                successor.heuristic(),
                start.heuristic(),
                "zero-day decisions must not change the research bound"
            );
        }
    }

    #[test]
    fn cloned_state_has_same_compact_identity() {
        let start = tech_fixture(10);
        let rebuilt = Vic3Node::new(start.state().clone(), start.goal().clone(), start.config());
        assert_eq!(start, rebuilt);
        assert_eq!(start.fingerprint(), rebuilt.fingerprint());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// I7 on the production goal-DAG relaxation: the estimate never
        /// exceeds the true remaining cost on reachable research formulas.
        #[test]
        fn goal_dag_bound_is_admissible_for_research_formulas(
            research_days in any::<u16>(),
            conjunction in any::<bool>(),
        ) {
            let source = if conjunction {
                "has_tech(railways) && has_tech(nitroglycerin)"
            } else {
                "has_tech(railways) || has_tech(nitroglycerin)"
            };
            let start = Vic3Node::new(
                PlanningState::default(),
                compile(source).unwrap(),
                SimConfig {
                    research_days,
                    ..SimConfig::default()
                },
            );
            let (path, cost) = shortest_path::<_, PairingHeap<_, _>>(&start)
                .expect("research formula is reachable");
            prop_assert!(start.heuristic() <= cost);
            for node in path {
                let remaining = shortest_path::<_, PairingHeap<_, _>>(&node)
                    .expect("node on a solution path remains reachable")
                    .1;
                prop_assert!(node.heuristic() <= remaining);
            }
        }
    }
}
