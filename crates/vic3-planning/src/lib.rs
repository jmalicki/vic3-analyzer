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

pub mod construction;
pub mod goals;
pub mod military;
pub mod plan;
pub mod sim;
pub mod tech;
pub mod tracks;
pub mod world;

#[cfg(test)]
mod test_support;

pub use construction::{
    allocation_cap_points_per_day, construction_add_for_cs_building, construction_eta_days,
    construction_points_per_day_from_save, construction_points_per_day_from_sectors,
    construction_points_per_day_from_world, construction_points_per_day_per_job,
    construction_sector_levels, construction_wait_days, construction_wait_target,
    government_construction_share_from_laws, max_parallel_construction_jobs,
    max_weekly_construction_progress, national_construction_points_per_day,
    sync_construction_points_per_day, unused_government_construction_points_per_day,
    ConstructionEtaMode, ConstructionSlots, MissingConstructionSectorPm,
    BASE_MAX_WEEKLY_CONSTRUCTION_PROGRESS, BUILDING_CONSTRUCTION_SECTOR,
    LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY,
};
pub use goals::{
    compile, evaluate, gaps, gaps_with_defs, parse, Goal, GoalError, InterestKind, Rel,
    SimpleSubgoal, COLONIZE_ARMY_THRESHOLD, COLONIZE_NAVY_THRESHOLD, COLONIZE_QUININE_TECH,
    COLONIZE_TECH, DECLARE_WAR_ARMY_THRESHOLD, DECLARE_WAR_MUNITIONS_PRICE_CEILING,
    LAW_COLONIAL_EXPLOITATION, LAW_COLONIAL_RESETTLEMENT,
};
pub use military::{
    army_buildings_fully_staffed, army_pp_from_buildings, is_barracks_building,
    is_military_planning_building, is_naval_admin_building, is_shipyard_building,
    military_buildings_fully_staffed, navy_buildings_fully_staffed, navy_pp_from_buildings,
    recompute_army_pp, recompute_navy_pp, MilBuildingKind, ModeledMilBuilding, UnitCombatStats,
    BUILDING_BARRACKS, BUILDING_NAVAL_ADMIN, BUILDING_SHIPYARD, BUILDING_SHIPYARD_ALT,
    MIL_INPUT_PRICE_FACTOR, STAFFING_EPS,
};
pub use plan::{
    compare, plan, plan_with_economy, ActionDiff, AnalysisRecord, CompareResult, GapDiff,
    GapStatus, PlanError, PlanOpts, PlanResult, PlanStep, PriceDelta, TimedEdge, TimedGraph,
    TimedNode, Vic3Node,
};
pub use sim::{
    apply_action, successors, successors_for_simple_subgoals, successors_with_economy, Action,
    EconomyContext, SimConfig, Successor,
};
pub use tech::{
    expand_tech_gap_simple_subgoals, missing_tech_closure, tech_prereqs_satisfied,
    tech_research_cost,
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
