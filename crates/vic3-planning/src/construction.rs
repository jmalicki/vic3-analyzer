//! Compact construction capacity model for planning.
//!
//! # Units
//!
//! | Quantity | Unit | Where |
//! | --- | --- | --- |
//! | Construction work | **construction points** (opaque game work units) | queue `remaining`, defs `required_construction`, [`SimConfig::default_construction_cost`] |
//! | National throughput | **construction points / day** | [`PlanningState::construction_points_per_day`] |
//! | Per-job feed | **construction points / day** | [`construction_points_per_day_per_job`] |
//! | Duration | **calendar days** | [`construction_wait_days`] ≈ `ceil(points / points_per_day)` |
//!
//! So “construction rate” in this crate always means **points per day**, never
//! a dimensionless multiplier or a money cost. Money and construction-goods
//! demand are out of scope (see module limitations below).
//!
//! # Role
//!
//! Victoria 3 allocates a national construction pool across queued building
//! levels. This module is the planner's **compact** version of that idea:
//!
//! - Throughput comes from Construction Sector levels (plus a small base).
//! - Each active job is capped at [`crate::sim::SimConfig::max_construction_allocation`]
//!   points per day; leftover throughput fills later queue entries.
//! - Wait edges advance to the soonest completion under that split.
//! - Construction Sector itself can appear as a means-to-an-end candidate so
//!   A* may invest in capacity before later goal-relevant builds.
//!
//! # Limitations
//!
//! - No per-state share of the national pool (sim queues often omit `state_id`).
//! - No construction-goods buy orders in the price solver and no treasury drain
//!   for those goods.
//! - No full Paradox script-value cost tables beyond loaded
//!   `required_construction`.
//!
//! See [`docs/planning.md`](../../../docs/planning.md).

use std::collections::BTreeSet;

use vic3_load::{Save, WorldSnapshot};
use vic3_prices::World;

use crate::goals::{Atom, Rel};
use crate::sim::{EconomyContext, SimConfig};
use crate::world::PlanningState;

/// Government Construction Sector building type id.
///
/// Levels of this building raise [`PlanningState::construction_points_per_day`]
/// via [`construction_points_per_day_from_sectors`] /
/// [`sync_construction_points_per_day`].
pub const BUILDING_CONSTRUCTION_SECTOR: &str = "building_construction_sector";

/// Default national throughput (points/day) with zero Construction Sectors.
///
/// Kept in sync with [`SimConfig::default`]'s `base_construction_capacity` so
/// load-time projection matches a default sim config.
pub const LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY: f64 = 1.0;

/// Extra national throughput (points/day) per Construction Sector level at load.
///
/// Kept in sync with [`SimConfig::default`]'s `construction_per_cs_level`.
pub const LOAD_CONSTRUCTION_POINTS_PER_DAY_PER_SECTOR_LEVEL: f64 = 5.0;

/// Convert Construction Sector levels into national throughput (**points/day**).
///
/// Uses the load-time defaults
/// ([`LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY`],
/// [`LOAD_CONSTRUCTION_POINTS_PER_DAY_PER_SECTOR_LEVEL`]). Non-finite or
/// non-positive results fall back to the base throughput.
///
/// # Examples
///
/// ```
/// use vic3_planning::construction::construction_points_per_day_from_sector_levels;
/// assert!((construction_points_per_day_from_sector_levels(0.0) - 1.0).abs() < 1e-9);
/// assert!((construction_points_per_day_from_sector_levels(2.0) - 11.0).abs() < 1e-9); // 1 + 5*2
/// ```
pub fn construction_points_per_day_from_sector_levels(sector_levels: f64) -> f64 {
    let points_per_day = LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY
        + LOAD_CONSTRUCTION_POINTS_PER_DAY_PER_SECTOR_LEVEL * sector_levels.max(0.0);
    if points_per_day.is_finite() && points_per_day > 0.0 {
        points_per_day
    } else {
        LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY
    }
}

/// Project national construction throughput (**points/day**) from a save.
///
/// Sums [`BUILDING_CONSTRUCTION_SECTOR`] levels in states owned by `country_id`,
/// then applies [`construction_points_per_day_from_sector_levels`].
pub fn construction_points_per_day_from_save(save: &Save, country_id: u32) -> f64 {
    let mut owned_states: BTreeSet<u32> = save
        .states
        .iter_present()
        .filter_map(|(id, state)| (state.country == Some(country_id)).then_some(id))
        .collect();
    if owned_states.is_empty() {
        if let Some((_, country)) = save.countries().find(|(id, _)| *id == country_id) {
            owned_states.extend(country.states.iter().copied());
        }
    }
    let levels: f64 = save
        .building_manager()
        .iter_present()
        .filter_map(|(_, building)| {
            let state = building.state?;
            if !owned_states.contains(&state) {
                return None;
            }
            if building.building != BUILDING_CONSTRUCTION_SECTOR {
                return None;
            }
            Some(f64::from(building.level.max(0)))
        })
        .sum();
    construction_points_per_day_from_sector_levels(levels)
}

/// Project national construction throughput (**points/day**) from a [`World`].
///
/// Same formula as [`construction_points_per_day_from_save`], using compact
/// world buildings owned by `country_id`.
pub fn construction_points_per_day_from_world(world: &World, country_id: u32) -> f64 {
    let owned_states: BTreeSet<u32> = world
        .states
        .iter()
        .filter_map(|state| (state.country == Some(country_id)).then_some(state.id))
        .collect();
    let levels: f64 = world
        .buildings
        .iter()
        .filter(|building| {
            building.building == BUILDING_CONSTRUCTION_SECTOR
                && building
                    .state
                    .is_some_and(|state_id| owned_states.contains(&state_id))
        })
        .map(|building| building.level.max(0.0))
        .sum();
    construction_points_per_day_from_sector_levels(levels)
}

/// Whether open goal atoms imply the planner may need to construct buildings.
///
/// Useful for docs / future gates. Construction Sector means-to-an-end now keys
/// off a non-empty candidate set (after direct/mil adds) rather than this alone.
#[allow(dead_code)] // kept for callers / future buildability filters
pub fn atoms_need_construction(atoms: &[Atom]) -> bool {
    atoms.iter().any(|atom| {
        matches!(
            atom,
            Atom::GoodPrice { .. }
                | Atom::Gdp {
                    rel: Rel::Ge | Rel::Gt | Rel::Eq,
                    ..
                }
                | Atom::ArmyPower { .. }
                | Atom::NavyPower { .. }
        )
    })
}

/// Sum Construction Sector levels on the planning branch.
///
/// When `economy` is present, reads levels from
/// [`EconomyContext::apply_planning_to_world`] (base world plus `building_level_deltas`).
/// Without economy, falls back to summing CS entries in
/// [`PlanningState::building_level_deltas`].
pub fn construction_sector_levels(state: &PlanningState, economy: Option<&EconomyContext>) -> f64 {
    if let Some(economy) = economy {
        return economy
            .apply_planning_to_world(state)
            .buildings
            .iter()
            .filter(|b| b.building == BUILDING_CONSTRUCTION_SECTOR)
            .map(|b| b.level.max(0.0))
            .sum();
    }
    f64::from(
        state
            .building_level_deltas
            .iter()
            .filter(|((building, _), _)| building == BUILDING_CONSTRUCTION_SECTOR)
            .map(|(_, levels)| *levels)
            .sum::<u32>(),
    )
}

/// National construction throughput (**points/day**) from CS levels and config.
///
/// `points/day = base_construction_capacity + construction_per_cs_level * levels`.
/// Non-finite or non-positive results fall back to at least one point of base
/// throughput so progress never stalls.
pub fn construction_points_per_day_from_sectors(
    state: &PlanningState,
    economy: Option<&EconomyContext>,
    config: SimConfig,
) -> f64 {
    let levels = construction_sector_levels(state, economy);
    let points_per_day = f64::from(config.base_construction_capacity)
        + f64::from(config.construction_per_cs_level) * levels;
    if points_per_day.is_finite() && points_per_day > 0.0 {
        points_per_day
    } else {
        f64::from(config.base_construction_capacity.max(1))
    }
}

/// Refresh [`PlanningState::construction_points_per_day`] from current CS levels.
///
/// Call after a Construction Sector level completes so subsequent waits use the
/// higher throughput. Uses [`construction_points_per_day_from_sectors`] with the
/// live economy world.
pub fn sync_construction_points_per_day(
    state: &mut PlanningState,
    economy: &EconomyContext,
    config: SimConfig,
) {
    state.construction_points_per_day =
        construction_points_per_day_from_sectors(state, Some(economy), config);
}

/// How many construction jobs may receive points under current throughput.
///
/// `max(1, floor(points_per_day / allocation_cap_points_per_day))`. A zero or
/// non-finite national throughput still yields one slot so a lone job can
/// progress.
pub fn max_parallel_construction_jobs(
    construction_points_per_day: f64,
    max_points_per_day_per_job: u16,
) -> usize {
    let cap = f64::from(max_points_per_day_per_job.max(1));
    if !construction_points_per_day.is_finite() || construction_points_per_day <= 0.0 {
        return 1;
    }
    ((construction_points_per_day / cap).floor() as usize).max(1)
}

/// True when compact parallel construction slots are full.
///
/// [`crate::sim::Action::QueueBuildingLevel`] is rejected while this holds so
/// the search does not enqueue deeper than throughput can actively feed.
pub fn construction_queue_full(state: &PlanningState, config: SimConfig) -> bool {
    let max = max_parallel_construction_jobs(
        state.construction_points_per_day,
        config.max_construction_allocation,
    );
    state.constructions.len() >= max
}

/// Per-job construction throughput (**points/day**) from the national pool.
///
/// Walks [`PlanningState::constructions`] in order. Each of the first
/// [`max_parallel_construction_jobs`] entries receives
/// `min(allocation_cap, remaining_pool)` until the pool is exhausted. Later
/// entries get `0.0` (queued but idle).
///
/// The returned slice length always matches `state.constructions.len()` so it
/// can be passed to [`PlanningState::tick_parallel_tracks`].
pub fn construction_points_per_day_per_job(state: &PlanningState, config: SimConfig) -> Vec<f64> {
    let national = state.construction_points_per_day;
    let alloc_cap = f64::from(config.max_construction_allocation.max(1));
    let max_jobs = max_parallel_construction_jobs(national, config.max_construction_allocation);
    let mut remaining = if national.is_finite() && national > 0.0 {
        national
    } else {
        0.0
    };
    let mut out = vec![0.0; state.constructions.len()];
    for points_per_day in out.iter_mut().take(max_jobs) {
        if remaining <= 0.0 {
            break;
        }
        let take = alloc_cap.min(remaining);
        *points_per_day = take;
        remaining -= take;
    }
    out
}

/// Days, building id, and placement state for the soonest active construction completion.
///
/// Uses [`construction_points_per_day_per_job`]. When `remaining` is set (save
/// in-flight work or def `required_construction`), ETA is
/// [`crate::tracks::days_for_work`] (`ceil(points / points_per_day)`). Otherwise
/// falls back to [`SimConfig::construction_days`]. Ties break by building id
/// ascending, then `state_id`.
///
/// Returns [`None`] when every active feed is non-positive and remaining work
/// is known (no finite ETA), or when the queue is empty.
pub fn construction_wait_target(
    state: &PlanningState,
    config: SimConfig,
) -> Option<(u16, String, Option<u32>)> {
    let feeds = construction_points_per_day_per_job(state, config);
    let mut best: Option<(u16, String, Option<u32>)> = None;
    for (job, points_per_day) in state.constructions.iter().zip(feeds.iter().copied()) {
        if points_per_day <= 0.0 {
            continue;
        }
        let days = match job.remaining.filter(|v| v.is_finite() && *v >= 0.0) {
            Some(work_points) => {
                let days = crate::tracks::days_for_work(work_points, points_per_day)?;
                u16::try_from(days).unwrap_or(u16::MAX)
            }
            None => config.construction_days,
        };
        let replace = match &best {
            None => true,
            Some((best_days, best_building, best_state)) => {
                days < *best_days
                    || (days == *best_days && job.building < *best_building)
                    || (days == *best_days
                        && job.building == *best_building
                        && job.state_id < *best_state)
            }
        };
        if replace {
            best = Some((days, job.building.clone(), job.state_id));
        }
    }
    best
}

/// Calendar days until the soonest construction completion (heuristic / tests).
///
/// Thin wrapper over [`construction_wait_target`] that drops building / state.
pub fn construction_wait_days(state: &PlanningState, config: SimConfig) -> Option<u16> {
    construction_wait_target(state, config).map(|(days, _, _)| days)
}

/// True when `building` (+ optional state) has a queue row with non-positive finite `remaining`.
///
/// Authorizes zero-day [`crate::sim::Event::BuildingCompleted`] waits after a
/// prior tick drained the job.
pub fn construction_work_complete(
    state: &PlanningState,
    building: &str,
    state_id: Option<u32>,
) -> bool {
    state.constructions.iter().any(|row| {
        row.building == building
            && state_id
                .map(|want| row.state_id == Some(want) || row.state_id.is_none())
                .unwrap_or(true)
            && row
                .remaining
                .is_some_and(|rem| rem.is_finite() && rem <= 0.0)
    })
}

/// Fill missing queue `remaining` (construction points) from
/// [`SimConfig::default_construction_cost`].
///
/// Save-loaded rows may omit work points; synthesizing them keeps allocation
/// ticks and ETA well-defined.
pub fn ensure_construction_work_points(state: &mut PlanningState, config: SimConfig) {
    let cost = f64::from(config.default_construction_cost);
    for job in &mut state.constructions {
        if job.remaining.is_none() {
            job.remaining = Some(cost);
        }
    }
}

/// Insert [`BUILDING_CONSTRUCTION_SECTOR`] into `candidates` when appropriate.
///
/// Emits CS only as a **means-to-an-end** meta lever: some other build type is
/// already a candidate (`!candidates.is_empty()`). Per-state level caps are
/// applied when expanding types to placement states.
pub fn maybe_add_construction_sector_candidate(
    _state: &PlanningState,
    candidates: &mut BTreeSet<String>,
    _cap: u16,
) {
    if candidates.is_empty() {
        return;
    }
    candidates.insert(BUILDING_CONSTRUCTION_SECTOR.to_string());
}

/// Construction points to store when enqueueing a building level.
///
/// Prefers defs [`vic3_defs::BuildingType::required_construction`] when finite
/// and non-negative; otherwise [`SimConfig::default_construction_cost`].
pub fn construction_work_points_for_enqueue(
    building: &str,
    economy: &EconomyContext,
    config: SimConfig,
) -> Option<f64> {
    economy
        .defs
        .buildings
        .get(building)
        .and_then(|b| b.required_construction)
        .filter(|c| c.is_finite() && *c >= 0.0)
        .or(Some(f64::from(config.default_construction_cost)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::SimConfig;
    use crate::world::{ConstructionQueueKind, PlanningConstruction, PlanningParts, PlanningState};
    use std::collections::BTreeMap;
    use vic3_defs::{BuildingType, GameDefs, GoodsVec};
    use vic3_prices::{SolveOpts, WorldBuilding, WorldCountry, WorldState};

    #[test]
    fn points_per_day_from_sector_levels_matches_load_defaults() {
        assert!((construction_points_per_day_from_sector_levels(0.0) - 1.0).abs() < 1e-9);
        assert!((construction_points_per_day_from_sector_levels(1.0) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn higher_points_per_day_shortens_wait() {
        let job = || PlanningConstruction {
            order_id: 1,
            queue: ConstructionQueueKind::Government,
            state_id: None,
            building: "building_logging_camp".into(),
            remaining: Some(100.0),
        };
        let slow = PlanningState::from_parts(PlanningParts {
            constructions: vec![job()],
            queued_building: Some("building_logging_camp".into()),
            construction_points_per_day: 5.0,
            ..PlanningParts::default()
        });
        let fast = PlanningState::from_parts(PlanningParts {
            constructions: vec![job()],
            queued_building: Some("building_logging_camp".into()),
            construction_points_per_day: 20.0,
            ..PlanningParts::default()
        });
        let config = SimConfig::default();
        assert_eq!(construction_wait_days(&slow, config), Some(20));
        assert_eq!(construction_wait_days(&fast, config), Some(5));
    }

    #[test]
    fn parallel_allocation_splits_points_per_day() {
        let state = PlanningState::from_parts(PlanningParts {
            constructions: vec![
                PlanningConstruction {
                    order_id: 1,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_a".into(),
                    remaining: Some(50.0),
                },
                PlanningConstruction {
                    order_id: 2,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_b".into(),
                    remaining: Some(50.0),
                },
            ],
            queued_building: Some("building_a".into()),
            construction_points_per_day: 10.0,
            ..PlanningParts::default()
        });
        let config = SimConfig {
            max_construction_allocation: 5,
            ..SimConfig::default()
        };
        assert_eq!(max_parallel_construction_jobs(10.0, 5), 2);
        let feeds = construction_points_per_day_per_job(&state, config);
        assert_eq!(feeds, vec![5.0, 5.0]);
        assert_eq!(construction_wait_days(&state, config), Some(10));
    }

    #[test]
    fn maybe_add_sector_requires_existing_build_path() {
        let state = PlanningState::default();
        let mut empty = BTreeSet::new();
        maybe_add_construction_sector_candidate(&state, &mut empty, 10);
        assert!(empty.is_empty(), "no CS without another candidate");

        let mut with_logging = BTreeSet::from(["building_logging_camp".into()]);
        maybe_add_construction_sector_candidate(&state, &mut with_logging, 10);
        assert!(with_logging.contains(BUILDING_CONSTRUCTION_SECTOR));
    }

    #[test]
    fn sync_points_per_day_after_cs_level() {
        let mut defs = GameDefs::default();
        defs.buildings.insert(
            BUILDING_CONSTRUCTION_SECTOR.into(),
            BuildingType {
                id: BUILDING_CONSTRUCTION_SECTOR.into(),
                group: None,
                city_type: None,
                production_method_groups: Vec::new(),
                required_construction: Some(10.0),
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
                building: BUILDING_CONSTRUCTION_SECTOR.into(),
                level: 0.0,
                staffing: 0.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            }],
            frozen_buy: GoodsVec::from_vec(vec![]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let config = SimConfig {
            base_construction_capacity: 1,
            construction_per_cs_level: 5,
            ..SimConfig::default()
        };
        let mut state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            construction_points_per_day: 1.0,
            building_level_deltas: BTreeMap::from([((BUILDING_CONSTRUCTION_SECTOR.into(), 1), 1)]),
            ..PlanningParts::default()
        });
        sync_construction_points_per_day(&mut state, &economy, config);
        // apply_planning_to_world applies +1 level → CS level 1 → 1 + 5*1 = 6 points/day
        assert!((state.construction_points_per_day - 6.0).abs() < 1e-9);
    }
}
