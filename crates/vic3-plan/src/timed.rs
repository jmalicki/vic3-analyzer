//! Toy timed graphs for generic A* (phase 9a).
//!
//! Proves the pathfinding contract before Vic3 wiring:
//! - intern key is [`TimedNode`] (`u32` id + [`Arc`] topology) — Eq/Hash use **id only**
//! - decision edges cost 0; wait edges cost positive days
//! - admissible `h` via reverse-topo DP on forward DAGs (**I7**)
//! - identical compact nodes ⇒ identical hash (**I8**)
//!
//! Production search uses [`crate::Vic3Node`] instead.

use crate::pathfinding::SearchNode;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One outgoing edge from a timed-graph node.
///
/// Decision edges cost 0 days (queue a tech, switch a PM, …). Wait edges cost a
/// positive number of days (event-wait until a clock fires).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimedEdge {
    /// Instant decision. Cost 0 days.
    Decision { to: u32 },
    /// Event-wait. `days` is a positive integer.
    Wait { to: u32, days: u32 },
}

impl TimedEdge {
    fn new(to: u32, days: u32) -> Self {
        if days == 0 {
            Self::Decision { to }
        } else {
            Self::Wait { to, days }
        }
    }

    /// Destination node id.
    pub fn to(self) -> u32 {
        match self {
            Self::Decision { to } | Self::Wait { to, .. } => to,
        }
    }

    /// Integer days to traverse this edge.
    pub fn days(self) -> u32 {
        match self {
            Self::Decision { .. } => 0,
            Self::Wait { days, .. } => days,
        }
    }
}

/// Shared topology for a toy timed graph (DAG in all P9a fixtures).
///
/// Edges are stored as adjacency lists. The admissible remaining-days estimate
/// `h` is exact on a forward DAG (`from < to` for every edge), computed by
/// reverse-topological DP — not a third A* loop.
#[derive(Debug)]
pub struct TimedGraph {
    successors: Vec<Vec<TimedEdge>>,
    is_goal: Vec<bool>,
    /// Admissible remaining days from each node. Exact on forward DAGs.
    h: Vec<u32>,
}

impl TimedGraph {
    /// Build a graph with `node_count` nodes.
    ///
    /// `edges` are `(from, to, days)`. A `days` of 0 is a decision; otherwise a
    /// wait. `goals` are node ids at which search terminates.
    pub fn new(node_count: usize, edges: &[(u32, u32, u32)], goals: &[u32]) -> Arc<Self> {
        assert!(node_count > 0, "timed graph needs at least one node");
        let mut successors = vec![Vec::new(); node_count];
        for &(from, to, days) in edges {
            assert!(
                (from as usize) < node_count && (to as usize) < node_count,
                "edge {from}->{to} out of range for {node_count} nodes"
            );
            successors[from as usize].push(TimedEdge::new(to, days));
        }
        let mut is_goal = vec![false; node_count];
        for &g in goals {
            assert!(
                (g as usize) < node_count,
                "goal {g} out of range for {node_count} nodes"
            );
            is_goal[g as usize] = true;
        }
        let h = remaining_days_lower_bound(&successors, &is_goal);
        Arc::new(Self {
            successors,
            is_goal,
            h,
        })
    }

    /// Hand-computed fixture: queue (0d) then wait 4d beats waiting 7d, beats a 10d detour.
    ///
    /// ```text
    /// 0 --0d--> 1 --4d--> 3 (goal)     cost 4  ← unique shortest
    /// 0 --7d--> 3                      cost 7
    /// 1 --0d--> 2 --10d--> 3           cost 10
    /// ```
    pub fn queue_then_wait() -> Arc<Self> {
        Self::new(
            4,
            &[(0, 1, 0), (0, 3, 7), (1, 3, 4), (1, 2, 0), (2, 3, 10)],
            &[3],
        )
    }

    /// Diamond: two waits of 3 and 5 after a shared 0-day decision, plus a slow 9d wait.
    pub fn diamond() -> Arc<Self> {
        Self::new(
            4,
            &[(0, 1, 0), (1, 2, 3), (1, 3, 5), (0, 3, 9), (2, 3, 0)],
            &[3],
        )
    }

    /// All-decision path of 0-day edges to the goal (shortest cost 0).
    pub fn all_decisions() -> Arc<Self> {
        Self::new(3, &[(0, 1, 0), (1, 2, 0), (0, 2, 0)], &[2])
    }

    /// Forward timed DAG: a chain `0→1→…→n-1` plus extra forward edges.
    ///
    /// `chain_costs.len()` must be `node_count - 1`. Extra edges with `from >= to`
    /// are skipped so the result stays a DAG. The last node is the unique goal.
    pub fn forward_dag(
        node_count: usize,
        chain_costs: &[u32],
        extras: &[(u32, u32, u32)],
    ) -> Arc<Self> {
        assert!(node_count >= 2, "DAG needs a start and a goal");
        assert_eq!(chain_costs.len(), node_count - 1);
        let mut edges: Vec<(u32, u32, u32)> = chain_costs
            .iter()
            .enumerate()
            .map(|(i, &days)| (i as u32, i as u32 + 1, days))
            .collect();
        for &(from, to, days) in extras {
            if from < to && (to as usize) < node_count {
                edges.push((from, to, days));
            }
        }
        Self::new(node_count, &edges, &[(node_count as u32) - 1])
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.successors.len()
    }

    /// Outgoing edges from `id`.
    pub fn edges(&self, id: u32) -> &[TimedEdge] {
        &self.successors[id as usize]
    }

    /// Whether `id` is a goal.
    pub fn is_goal(&self, id: u32) -> bool {
        self.is_goal[id as usize]
    }

    /// Admissible remaining-days estimate at `id`.
    pub fn heuristic(&self, id: u32) -> u32 {
        self.h[id as usize]
    }

    /// Cheapest edge cost from `from` to `to`, if any.
    pub fn step_days(&self, from: u32, to: u32) -> Option<u32> {
        self.edges(from)
            .iter()
            .filter(|e| e.to() == to)
            .map(|e| e.days())
            .min()
    }
}

/// Reverse-id DP remaining days.
///
/// On a forward DAG every successor has a greater id, so this is the true
/// remaining cost. If a back-edge is present, uncomputed successors contribute
/// 0 and the value may underestimate — still admissible, never an overestimate
/// from using `u32::MAX` sentinels.
fn remaining_days_lower_bound(successors: &[Vec<TimedEdge>], is_goal: &[bool]) -> Vec<u32> {
    let n = successors.len();
    let mut h = vec![0u32; n];
    for i in (0..n).rev() {
        if is_goal[i] {
            h[i] = 0;
            continue;
        }
        let mut best: Option<u32> = None;
        for edge in &successors[i] {
            let cand = edge.days().saturating_add(h[edge.to() as usize]);
            best = Some(best.map_or(cand, |b| b.min(cand)));
        }
        h[i] = best.unwrap_or(0);
    }
    h
}

/// Compact search node: intern-map key is the `u32` id.
#[derive(Clone, Debug)]
pub struct TimedNode {
    id: u32,
    graph: Arc<TimedGraph>,
}

impl TimedNode {
    /// Node `id` in `graph`.
    pub fn at(id: u32, graph: Arc<TimedGraph>) -> Self {
        assert!(
            (id as usize) < graph.node_count(),
            "node {id} out of range for {} nodes",
            graph.node_count()
        );
        Self { id, graph }
    }

    /// Compact identity (intern-map key).
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Shared topology.
    pub fn graph(&self) -> &Arc<TimedGraph> {
        &self.graph
    }
}

impl PartialEq for TimedNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TimedNode {}

impl Hash for TimedNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl SearchNode for TimedNode {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, u32)> {
        self.graph
            .edges(self.id)
            .iter()
            .map(|edge| {
                (
                    Self {
                        id: edge.to(),
                        graph: Arc::clone(&self.graph),
                    },
                    edge.days(),
                )
            })
            .collect()
    }

    fn is_goal(&self) -> bool {
        self.graph.is_goal(self.id)
    }

    fn heuristic(&self) -> u32 {
        self.graph.heuristic(self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::{shortest_path, shortest_path_lazy};
    use proptest::prelude::*;
    use rust_advanced_heaps::pairing::PairingHeap;
    use rust_advanced_heaps::simple_binary::SimpleBinaryHeap;
    use std::hash::DefaultHasher;
    use std::sync::Arc;

    fn astar(start: &TimedNode) -> Option<(Vec<TimedNode>, u32)> {
        shortest_path::<_, PairingHeap<_, _>>(start)
    }

    fn lazy(start: &TimedNode) -> Option<(Vec<TimedNode>, u32)> {
        shortest_path_lazy::<_, SimpleBinaryHeap<_, _>>(start)
    }

    fn compact_hash(node: &TimedNode) -> u64 {
        let mut hasher = DefaultHasher::new();
        node.hash(&mut hasher);
        hasher.finish()
    }

    /// Remaining days along `path` from index `i`, using cheapest connecting edges.
    fn remaining_on_path(graph: &TimedGraph, path: &[TimedNode], cost: u32, i: usize) -> u32 {
        let mut g = 0u32;
        for w in 0..i {
            g += graph
                .step_days(path[w].id(), path[w + 1].id())
                .expect("path nodes are adjacent");
        }
        cost.saturating_sub(g)
    }

    #[test]
    fn known_shortest_path_queue_then_wait() {
        let graph = TimedGraph::queue_then_wait();
        let start = TimedNode::at(0, Arc::clone(&graph));
        let (path, cost) = astar(&start).expect("goal is reachable");
        assert_eq!(cost, 4, "hand-computed optimum is 4 days");
        let ids: Vec<u32> = path.iter().map(TimedNode::id).collect();
        assert_eq!(ids, vec![0, 1, 3]);
        assert_eq!(start.heuristic(), 4);
    }

    #[test]
    fn diamond_shortest_is_three_days() {
        // 0 -0d-> 1 -5d-> 3 = 5, vs 0 -0d-> 1 -3d-> 2 -0d-> 3 = 3, vs 0 -9d-> 3 = 9.
        let start = TimedNode::at(0, TimedGraph::diamond());
        let (path, cost) = astar(&start).expect("goal is reachable");
        assert_eq!(cost, 3);
        let ids: Vec<u32> = path.iter().map(TimedNode::id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn all_decisions_cost_zero() {
        let start = TimedNode::at(0, TimedGraph::all_decisions());
        let (_, cost) = astar(&start).expect("goal is reachable");
        assert_eq!(cost, 0);
        assert_eq!(start.heuristic(), 0);
    }

    #[test]
    fn shortest_path_agrees_with_lazy_on_fixtures() {
        let fixtures = [
            TimedGraph::queue_then_wait(),
            TimedGraph::diamond(),
            TimedGraph::all_decisions(),
        ];
        for graph in fixtures {
            let start = TimedNode::at(0, graph);
            let pairing = astar(&start).map(|(_, c)| c);
            let baseline = lazy(&start).map(|(_, c)| c);
            assert_eq!(pairing, baseline);
        }
    }

    #[test]
    fn i8_identical_compact_node_same_hash() {
        let a = TimedNode::at(0, TimedGraph::queue_then_wait());
        let b = TimedNode::at(0, TimedGraph::queue_then_wait());
        assert_eq!(a, b);
        assert_eq!(compact_hash(&a), compact_hash(&b));
        assert_eq!(compact_hash(&a), compact_hash(&a.clone()));

        let other = TimedNode::at(1, TimedGraph::queue_then_wait());
        assert_ne!(a, other);
        assert_ne!(compact_hash(&a), compact_hash(&other));
    }

    #[test]
    fn i8_search_is_deterministic() {
        let graph = TimedGraph::queue_then_wait();
        let start = TimedNode::at(0, Arc::clone(&graph));
        let cost_a = astar(&start).expect("reachable").1;
        let cost_b = astar(&start).expect("reachable").1;
        let rebuilt = TimedNode::at(0, TimedGraph::queue_then_wait());
        let cost_c = astar(&rebuilt).expect("reachable").1;
        assert_eq!(cost_a, cost_b);
        assert_eq!(cost_a, cost_c);
        assert_eq!(cost_a, 4);

        let succs_a = start.successors();
        let succs_b = start.successors();
        assert_eq!(succs_a, succs_b);
    }

    fn arb_forward_dag() -> impl Strategy<Value = Arc<TimedGraph>> {
        (2usize..=8).prop_flat_map(|n| {
            let chain = prop::collection::vec(0u32..=10, n - 1);
            let extras = prop::collection::vec(any::<(u8, u8, u8)>(), 0..=12);
            (chain, extras).prop_map(move |(chain_costs, raw_extras)| {
                let extras: Vec<(u32, u32, u32)> = raw_extras
                    .into_iter()
                    .filter_map(|(a, b, c)| {
                        let from = (a as usize) % n;
                        let span = n.saturating_sub(from + 1);
                        if span == 0 {
                            return None;
                        }
                        let to = from + 1 + ((b as usize) % span);
                        Some((from as u32, to as u32, u32::from(c % 11)))
                    })
                    .collect();
                TimedGraph::forward_dag(n, &chain_costs, &extras)
            })
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn i7_heuristic_never_exceeds_remaining(graph in arb_forward_dag()) {
            let n = graph.node_count();
            let start = TimedNode::at(0, Arc::clone(&graph));
            let (path, cost) = astar(&start).expect("chain reaches the goal");
            prop_assert!(start.heuristic() <= cost);

            for (i, node) in path.iter().enumerate() {
                let remaining = remaining_on_path(&graph, &path, cost, i);
                prop_assert!(
                    node.heuristic() <= remaining,
                    "I7: h({}) = {} > remaining {}",
                    node.id(),
                    node.heuristic(),
                    remaining
                );
            }

            for id in 0..n as u32 {
                let node = TimedNode::at(id, Arc::clone(&graph));
                if let Some((_, true_remaining)) = astar(&node) {
                    prop_assert!(
                        node.heuristic() <= true_remaining,
                        "I7: h({id}) = {} > shortest_path {}",
                        node.heuristic(),
                        true_remaining
                    );
                }
            }
        }

        #[test]
        fn shortest_path_agrees_with_lazy_on_random_dags(graph in arb_forward_dag()) {
            let start = TimedNode::at(0, graph);
            let pairing = astar(&start).map(|(_, c)| c);
            let baseline = lazy(&start).map(|(_, c)| c);
            prop_assert_eq!(pairing, baseline);
        }
    }
}
