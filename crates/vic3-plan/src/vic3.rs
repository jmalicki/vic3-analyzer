//! A* node backed by the goal-relevant [`vic3_sim`] transition model.
//!
//! [`Vic3Node`] keeps the intern-map key cheap: equality and hashing inspect
//! only the state's precomputed `u64` fingerprint. The projected state and
//! immutable search inputs ride behind [`Arc`]s and are not deeply compared by
//! the pathfinder.

use crate::pathfinding::SearchNode;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use vic3_goals::{evaluate, Goal};
use vic3_sim::SimConfig;
use vic3_world::PlanningState;

/// Immutable inputs shared by every node in one planning search.
#[derive(Debug)]
struct SearchContext {
    goal: Goal,
    config: SimConfig,
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
        Self::with_context(state, Arc::new(SearchContext { goal, config }))
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

impl SearchNode for Vic3Node {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, Self::Cost)> {
        vic3_sim::successors(&self.state, &self.context.goal, self.context.config)
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

    /// Dijkstra for v1: zero is an admissible remaining-days lower bound.
    fn heuristic(&self) -> Self::Cost {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::{shortest_path, shortest_path_lazy};
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
            SimConfig { research_days },
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
    fn cloned_state_has_same_compact_identity() {
        let start = tech_fixture(10);
        let rebuilt = Vic3Node::new(start.state().clone(), start.goal().clone(), start.config());
        assert_eq!(start, rebuilt);
        assert_eq!(start.fingerprint(), rebuilt.fingerprint());
    }
}
