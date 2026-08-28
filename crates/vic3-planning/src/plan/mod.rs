//! Planner glue: A* over [`crate::sim`] successors + shared JSON types.
//!
//! # Stack
//!
//! ```text
//! PlanningState  →  Goal (compile)  →  sim successors  →  Vic3Node / A*
//!                                              ↘ gaps() / evaluate()
//! ```
//!
//! - Phase 9a toys: [`timed`] graphs with cheap [`timed::TimedNode`] ids
//! - Phase 9b: [`Vic3Node`] wires `rust_advanced_heaps::pathfinding` to sim
//! - [`plan`] / [`plan_with_economy`] (aliases) → [`PlanResult`] for CLI, wasm, SQL `plan()`
//!   — both require [`crate::sim::EconomyContext`] (use [`crate::sim::EconomyContext::empty`] for timing-only)
//!
//! # Why cheap intern keys
//!
//! `SearchNode: Clone + Eq + Hash` is stored in the pathfinder's HashMap.
//! Fat worlds must not be key bodies: [`TimedNode`] hashes a `u32` id;
//! [`Vic3Node`] hashes only [`crate::world::PlanningState::fingerprint`]. State,
//! goal, and economy ride behind [`std::rc::Rc`] (upgrade to [`std::sync::Arc`]
//! when search must be `Send + Sync`).
//!
//! # Heuristic (I7)
//!
//! [`Vic3Node`]'s heuristic is an admissible remaining-days DAG relaxation of
//! the compiled goal (AND = max child, OR = min, timed atoms = model durations,
//! fiscal/SoL/tax = 0). Exact on P9a forward DAGs; property-tested on research
//! formulas for Vic3 nodes.
//!
//! # Consumers
//!
//! UI presets and SQL `plan`/`gaps` TVFs pass the same DSL string and
//! [`PlanOpts`]-shaped options; atoms are unchanged end-to-end.
//!
//! Do **not** use crates.io `pathfinding`. See [`docs/planning.md`](../../../docs/planning.md).

/// Ensure the heaps crate stays in the graph (pathfinding API used in phase 9a).
pub use rust_advanced_heaps::pathfinding;

pub mod timed;
pub use timed::{TimedEdge, TimedGraph, TimedNode};

mod greedy;
mod progress_h;
pub use greedy::greedy_upper_bound;

mod vic3;
pub use vic3::Vic3Node;

mod pea;
pub use pea::{PeaNode, DEFAULT_PEA_BEAM};

/// Temporary A* expand logging (`VIC3_PLAN_TRACE=1`).
pub mod astar_trace;

mod result;
pub use result::{
    compare, plan, plan_with_economy, ActionDiff, AnalysisRecord, CompareResult, GapDiff,
    GapStatus, PlanError, PlanOpts, PlanResult, PlanStep, PriceDelta,
};

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_semver() {
        assert!(!super::version().is_empty());
    }
}
