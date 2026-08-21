//! Planning stack: compact world → goal DSL → successors → A*.
//!
//! ```text
//! PlanningState  →  Goal (compile)  →  sim successors  →  Vic3Node / A*
//!                                              ↘ gaps() / evaluate()
//! ```
//!
//! Modules mirror the former micro-crates (`vic3-world`, `vic3-goals`,
//! `vic3-sim`, `vic3-plan`) so call sites stay obvious. Facades share this
//! crate through [`vic3_api`](../vic3-api) / CLI / SQL.
//!
//! Do **not** use crates.io `pathfinding`. See [`docs/planning.md`](../../../docs/planning.md).

pub mod goals;
pub mod plan;
pub mod sim;
pub mod world;

pub use goals::{
    compile, evaluate, gaps, parse, Atom, Goal, GoalError, InterestKind, Rel,
    DECLARE_WAR_ARMY_THRESHOLD, DECLARE_WAR_MUNITIONS_PRICE_CEILING,
};
pub use plan::{
    compare, plan, plan_with_economy, ActionDiff, AnalysisRecord, CompareResult, GapDiff,
    GapStatus, PlanError, PlanOpts, PlanResult, PlanStep, PriceDelta, TimedEdge, TimedGraph,
    TimedNode, Vic3Node,
};
pub use sim::{
    apply_action, army_power_raise_target, successors, successors_for_atoms,
    successors_with_economy, Action, EconomyContext, SimConfig, Successor,
};
pub use world::{
    law_key, ConstructionQueueKind, PlanningParts, PlanningState, QueuedInterest, Save, Vic3Date,
    WorldError, ARMY_POWER_PROJECTION_UNKNOWN,
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
