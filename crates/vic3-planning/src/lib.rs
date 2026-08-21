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
//! Framework-shaped seams (tracks/ETA, DSL solvers) are documented in
//! [`docs/planning.md`](../../../docs/planning.md#framework-seams). Domain
//! timing should prefer [`tracks`] over ad hoc day constants when work/rate
//! data exists.
//!
//! Do **not** use crates.io `pathfinding`. See [`docs/planning.md`](../../../docs/planning.md).

pub mod goals;
pub mod military;
pub mod plan;
pub mod sim;
pub mod tracks;
pub mod world;

pub use goals::{
    compile, evaluate, gaps, parse, Atom, Goal, GoalError, InterestKind, Rel,
    COLONIZE_ARMY_THRESHOLD, COLONIZE_NAVY_THRESHOLD, COLONIZE_QUININE_TECH, COLONIZE_TECH,
    DECLARE_WAR_ARMY_THRESHOLD, DECLARE_WAR_MUNITIONS_PRICE_CEILING, LAW_COLONIAL_EXPLOITATION,
    LAW_COLONIAL_RESETTLEMENT,
};
pub use military::{
    army_buildings_fully_staffed, army_pp_from_buildings, is_barracks_building,
    is_military_planning_building, is_naval_admin_building, is_shipyard_building,
    military_buildings_fully_staffed, navy_buildings_fully_staffed, navy_pp_from_buildings,
    recompute_army_pp, recompute_navy_pp, ModeledMilBuilding, UnitCombatStats, BUILDING_BARRACKS,
    BUILDING_NAVAL_ADMIN, BUILDING_SHIPYARD, BUILDING_SHIPYARD_ALT, MIL_INPUT_PRICE_FACTOR,
    STAFFING_EPS,
};
pub use plan::{
    compare, plan, plan_with_economy, ActionDiff, AnalysisRecord, CompareResult, GapDiff,
    GapStatus, PlanError, PlanOpts, PlanResult, PlanStep, PriceDelta, TimedEdge, TimedGraph,
    TimedNode, Vic3Node,
};
pub use sim::{
    apply_action, successors, successors_for_atoms, successors_with_economy, Action,
    EconomyContext, SimConfig, Successor,
};
pub use tracks::{
    constant_rate_work, days_for_work, eta_days, eta_head_days, eta_prefix_days, next_completion,
    Backlog, Job, TrackId, TrackState, WorkerPool, CONSTANT_RATE,
};
pub use world::{
    law_key, ConstructionQueueKind, PlanningParts, PlanningState, QueuedInterest, Save, Vic3Date,
    WorldError, ARMY_POWER_PROJECTION_UNKNOWN, NAVY_POWER_PROJECTION_UNKNOWN,
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
