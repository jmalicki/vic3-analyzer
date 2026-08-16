//! A* node backed by the goal-relevant [`vic3_sim`] transition model.
//!
//! [`Vic3Node`] keeps the intern-map key cheap: equality and hashing inspect
//! only the state's precomputed `u64` fingerprint. The projected state and
//! immutable search inputs ride behind [`Arc`]s and are not deeply compared by
//! the pathfinder.

use crate::pathfinding::SearchNode;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use vic3_goals::{evaluate, Atom, Goal};
use vic3_sim::{EconomyContext, SimConfig, Successor};
use vic3_world::PlanningState;

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
            Some(economy) => vic3_sim::successors_with_economy(
                &self.state,
                &self.context.goal,
                self.context.config,
                economy,
            ),
            None => vic3_sim::successors(&self.state, &self.context.goal, self.context.config),
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
/// Atomic timing is currently known only for research. AND uses the longest
/// child (actions may overlap or satisfy multiple atoms), OR uses the cheapest
/// child, and NOT stays at zero. Missing technologies always contribute the
/// fixed research duration, even when an unrelated tech is queued: returning
/// zero there would drop the estimate over a zero-day queue edge and break
/// consistency for closed-node A*. This is deliberately a relaxation of the
/// real state graph, not a replacement for A*.
fn goal_timing_lower_bound(goal: &Goal, state: &PlanningState, config: SimConfig) -> u32 {
    let research_days = u32::from(config.research_days.max(1));
    match goal {
        Goal::And(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config))
            .max()
            .unwrap_or(0),
        Goal::Or(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config))
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

    /// Goal-DAG relaxation: exact for one research atom, conservative otherwise.
    fn heuristic(&self) -> Self::Cost {
        goal_timing_lower_bound(&self.context.goal, &self.state, self.context.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::{shortest_path, shortest_path_lazy};
    use proptest::prelude::*;
    use rust_advanced_heaps::pairing::PairingHeap;
    use rust_advanced_heaps::simple_binary::SimpleBinaryHeap;
    use vic3_goals::compile;
    use vic3_sim::Action;
    use vic3_world::PlanningParts;

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

        let first_edges = vic3_sim::successors(start.state(), start.goal(), start.config());
        assert!(matches!(
            first_edges.as_slice(),
            [vic3_sim::Successor {
                action: Action::QueueTech { tech },
                days: 0,
                ..
            }] if tech == "nitroglycerin"
        ));
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
