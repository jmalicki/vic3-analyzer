//! Compact construction capacity model for planning.
//!
//! # Units
//!
//! | Quantity | Unit | Where |
//! | --- | --- | --- |
//! | Construction work | **construction points** (opaque game work units) | queue `remaining`, defs `required_construction`, [`SimConfig::default_construction_cost`] |
//! | Government throughput | **construction points / day** | [`PlanningState::construction_points_per_day`] |
//! | Per-job feed | **construction points / day** | [`construction_points_per_day_per_job`] |
//! | Duration | **calendar days** | [`construction_wait_days`] ≈ `ceil(points / points_per_day)` |
//!
//! So “construction rate” in this crate always means **points per day**, never
//! a dimensionless multiplier or a money cost. Money and construction-goods
//! demand are out of scope (see module limitations below).
//!
//! # Role
//!
//! Victoria 3 allocates a **national** construction pool across queued building
//! levels, then splits that pool between **government** and **private** queues
//! via economic-system laws (`country_private_construction_allocation_mult`).
//! There is **no** geographic-state share of the national pool in the game model
//! we care about — only national throughput and the government/private split.
//! This module is the planner's **compact** version of that idea:
//!
//! - Throughput comes from Construction Sector levels × **required** CS PM
//!   `country_construction_add` (from defs). Missing/invalid CS PMs are errors,
//!   not silent iron-frame-shaped guesses.
//! - Only the **government** share feeds planner jobs; private queue rows are
//!   ignored for allocation / parallel slots.
//! - Each active government job is capped at max weekly construction progress
//!   ÷ 7 (base 10/week from game static modifiers + owned tech adds), unless
//!   [`SimConfig::max_construction_allocation`] overrides. Leftover government
//!   throughput fills later government queue entries.
//! - Wait edges advance to the soonest **fed** government completion.
//! - Heuristic ETA ([`construction_eta_days`]) defaults to time until a free
//!   government feed slot / usable capacity; explicit next-finish mode is for
//!   wait-with-spare-slots semantics.
//! - Construction Sector itself can appear as a means-to-an-end candidate so
//!   A* may invest in capacity before later goal-relevant builds.
//!
//! # Limitations / approximations
//!
//! - Government vs private uses a static economic-law → private-mult table
//!   (vanilla `01_economic_system.txt`); other modifiers that change private
//!   allocation are ignored.
//! - Per-job cap ignores building-group `construction_efficiency_*` and company
//!   bonuses beyond the tech table for `country_max_weekly_construction_progress_add`.
//! - CS throughput assumes full staffing (`workforce_scaled` as level-scaled).
//! - No construction-goods buy orders in the price solver and no treasury drain
//!   for those goods.
//! - No full Paradox script-value cost tables beyond loaded
//!   `required_construction`.
//!
//! See [`docs/planning.md`](../../../docs/planning.md).

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use vic3_defs::GameDefs;
use vic3_load::{Save, WorldSnapshot};
use vic3_prices::World;

use crate::goals::{Rel, SimpleSubgoal};
use crate::sim::{EconomyContext, SimConfig};
use crate::world::{ConstructionQueueKind, PlanningState};

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

/// Parallel government construction feed slots — always ≥ 1.
///
/// Encodes the queue-admissibility floor: at least one government job may be
/// enqueued (base capacity / zero-throughput slot count). Prefer this over raw
/// `usize` so call sites cannot forget the minimum.
///
/// **Note:** a slot count of one does not by itself feed work — when national
/// `construction_points_per_day` is ≤ 0, allocation still emits 0 points/day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConstructionSlots(NonZeroUsize);

impl ConstructionSlots {
    /// Single feed slot (base / fallback).
    pub const ONE: Self = Self(NonZeroUsize::MIN);

    /// From a count; zero or underflow → [`Self::ONE`].
    pub fn new(n: usize) -> Self {
        NonZeroUsize::new(n).map(Self).unwrap_or(Self::ONE)
    }

    /// Slot count as `usize` (≥ 1).
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

impl From<ConstructionSlots> for usize {
    fn from(slots: ConstructionSlots) -> Self {
        slots.get()
    }
}

impl std::fmt::Display for ConstructionSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Vanilla base `country_max_weekly_construction_progress_add` from
/// `00_code_static_modifiers.txt`.
pub const BASE_MAX_WEEKLY_CONSTRUCTION_PROGRESS: f64 = 10.0;

/// Construction Sector building lacks a defs PM that yields
/// `country_construction_add`.
///
/// CS throughput never invents iron-frame-shaped defaults: production methods
/// on the building must resolve against defs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "construction sector building {building_id} has no production method with \
     country_construction_add (methods={methods:?}); CS PMs are required"
)]
pub struct MissingConstructionSectorPm {
    pub building_id: u32,
    pub methods: Vec<String>,
}

/// Heuristic / wait ETA mode for construction timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstructionEtaMode {
    /// Time until government feed capacity / a free parallel slot is available.
    ///
    /// When government slots are full, this is the soonest fed completion (slot
    /// frees). When spare slots exist, this is one default-cost level at the
    /// leftover government feed rate — not “next finish”, so open GDP simple
    /// subgoals are not pinned to a sticky 1-day next-completion bound.
    CapacityOrSlot,
    /// Soonest actively-fed government construction completion.
    ///
    /// Use when an explicit wait advances with spare slots still open.
    NextFinish,
}

/// Convert Construction Sector buildings into national throughput (**points/day**).
///
/// Sums `level × country_construction_add` for owned CS buildings. Each positive
/// level **requires** a production method in `defs` with finite
/// `country_construction_add ≥ 0`. Zero CS levels →
/// [`LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY`] only (before government share).
fn national_points_per_day_from_cs_buildings<'a>(
    buildings: impl IntoIterator<Item = &'a vic3_prices::WorldBuilding>,
    defs: &GameDefs,
    base: f64,
) -> Result<f64, MissingConstructionSectorPm> {
    let mut from_buildings = 0.0;
    for building in buildings {
        let level = building.level.max(0.0);
        if level <= 0.0 {
            continue;
        }
        let add = construction_add_for_cs_building(building, defs)?;
        from_buildings += add * level;
    }
    let points_per_day = base + from_buildings;
    if points_per_day.is_finite() && points_per_day > 0.0 {
        Ok(points_per_day)
    } else {
        Ok(base.max(LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY))
    }
}

/// Resolve `country_construction_add` for one Construction Sector building.
///
/// Walks [`vic3_prices::WorldBuilding::production_methods`] (required, must be
/// valid against defs). First PM that grants a finite non-negative add wins.
pub fn construction_add_for_cs_building(
    building: &vic3_prices::WorldBuilding,
    defs: &GameDefs,
) -> Result<f64, MissingConstructionSectorPm> {
    for pm_id in &building.production_methods {
        if let Some(add) = defs
            .production_methods
            .get(pm_id)
            .and_then(|pm| pm.country_construction_add)
            .filter(|v| v.is_finite() && *v >= 0.0)
        {
            return Ok(add);
        }
    }
    Err(MissingConstructionSectorPm {
        building_id: building.id,
        methods: building.production_methods.clone(),
    })
}

fn expect_construction_add(building: &vic3_prices::WorldBuilding, defs: &GameDefs) -> f64 {
    construction_add_for_cs_building(building, defs).unwrap_or_else(|err| {
        panic!("{err}; fix the save/fixture so Construction Sector PMs resolve in defs")
    })
}

/// Government share of national construction (1 − private allocation mult).
///
/// Looks up vanilla economic-system laws on `laws`. Unknown / missing economic
/// law → `1.0` (all government) so tests without laws keep full throughput.
pub fn government_construction_share_from_laws<'a>(laws: impl IntoIterator<Item = &'a str>) -> f64 {
    let mut private = None;
    for law in laws {
        let key = crate::world::law_key(law);
        if let Some(mult) = private_construction_allocation_mult(&key) {
            private = Some(mult);
            break;
        }
    }
    match private {
        Some(p) if p.is_finite() => (1.0 - p).clamp(0.0, 1.0),
        _ => 1.0,
    }
}

fn private_construction_allocation_mult(law_key: &str) -> Option<f64> {
    // From game `common/laws/01_economic_system.txt`.
    Some(match law_key {
        "traditionalism" => 0.25,
        "interventionism" | "agrarianism" | "industry_banned" | "extraction_economy" => 0.5,
        "laissez_faire" => 0.75,
        "cooperative_ownership" => 0.35,
        "command_economy" => 0.1,
        _ => return None,
    })
}

fn owned_state_ids_from_save(save: &Save, country_id: u32) -> BTreeSet<u32> {
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
    owned_states
}

/// Project **government** construction throughput (**points/day**) from a save.
///
/// Owned Construction Sector buildings must list production methods that resolve
/// in `defs` to `country_construction_add`. Then scales by
/// [`government_construction_share_from_laws`].
pub fn construction_points_per_day_from_save(
    save: &Save,
    country_id: u32,
    defs: &GameDefs,
) -> Result<f64, MissingConstructionSectorPm> {
    let owned_states = owned_state_ids_from_save(save, country_id);
    let mut from_buildings = 0.0;
    for (id, building) in save.building_manager().iter_present() {
        let Some(state_id) = building.state else {
            continue;
        };
        if !owned_states.contains(&state_id) {
            continue;
        }
        if building.building != BUILDING_CONSTRUCTION_SECTOR {
            continue;
        }
        let level = f64::from(building.level.max(0));
        if level <= 0.0 {
            continue;
        }
        let methods = building.active_production_methods();
        let Some(type_id) = defs.building_index_of(&building.building) else {
            continue;
        };
        let world_building = vic3_prices::WorldBuilding {
            id,
            state: building.state,
            building_type_id: type_id,
            level,
            staffing: building.staffing.max(0.0),
            production_methods: methods,
            saved_inputs: Vec::new(),
            saved_outputs: Vec::new(),
        };
        from_buildings += construction_add_for_cs_building(&world_building, defs)? * level;
    }
    let national = LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY + from_buildings;
    let share = government_construction_share_from_laws(save.active_laws(country_id));
    let govt = national * share;
    if govt.is_finite() && govt > 0.0 {
        Ok(govt)
    } else {
        Ok(LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY)
    }
}

/// Project **government** construction throughput (**points/day**) from a [`World`].
///
/// Same PM requirements as [`construction_points_per_day_from_save`]. Applies
/// government share from the country's laws on `world`.
pub fn construction_points_per_day_from_world(
    world: &World,
    country_id: u32,
    defs: &GameDefs,
) -> Result<f64, MissingConstructionSectorPm> {
    let owned_states: BTreeSet<u32> = world
        .states
        .iter()
        .filter_map(|state| (state.country == Some(country_id)).then_some(state.id))
        .collect();
    let cs = world.buildings.iter().filter(|building| {
        building.type_script_id(defs) == BUILDING_CONSTRUCTION_SECTOR
            && building
                .state
                .is_some_and(|state_id| owned_states.contains(&state_id))
    });
    let national =
        national_points_per_day_from_cs_buildings(cs, defs, LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY)?;
    let laws = world
        .countries
        .iter()
        .find(|c| c.id == country_id)
        .map(|c| c.laws.iter().map(String::as_str))
        .into_iter()
        .flatten();
    let share = government_construction_share_from_laws(laws);
    let govt = national * share;
    if govt.is_finite() && govt > 0.0 {
        Ok(govt)
    } else {
        Ok(LOAD_BASE_CONSTRUCTION_POINTS_PER_DAY)
    }
}

/// Whether open simple subgoals imply the planner may need to construct buildings.
///
/// Useful for docs / future gates. Construction Sector means-to-an-end now keys
/// off a non-empty candidate set (after direct/mil adds) rather than this alone.
#[allow(dead_code)] // kept for callers / future buildability filters
pub fn simple_subgoals_need_construction(atoms: &[SimpleSubgoal]) -> bool {
    atoms.iter().any(|atom| {
        matches!(
            atom,
            SimpleSubgoal::GoodPrice { .. }
                | SimpleSubgoal::Gdp {
                    rel: Rel::Ge | Rel::Gt | Rel::Eq,
                    ..
                }
                | SimpleSubgoal::ArmyPower { .. }
                | SimpleSubgoal::NavyPower { .. }
        )
    })
}

/// Sum Construction Sector levels on the planning branch.
///
/// Reads levels from [`EconomyContext::apply_planning_to_world`] (base world
/// plus `building_level_deltas`).
pub fn construction_sector_levels(state: &PlanningState, economy: &EconomyContext) -> f64 {
    economy
        .apply_planning_to_world(state)
        .buildings
        .iter()
        .filter(|b| b.type_script_id(&economy.defs) == BUILDING_CONSTRUCTION_SECTOR)
        .map(|b| b.level.max(0.0))
        .sum()
}

/// National (pre-share) construction throughput from CS levels / **required** PMs.
///
/// Every positive-level Construction Sector building must resolve
/// `country_construction_add` in defs (see [`construction_add_for_cs_building`]).
pub fn national_construction_points_per_day(
    state: &PlanningState,
    economy: &EconomyContext,
    config: SimConfig,
) -> f64 {
    let base = f64::from(config.base_construction_capacity);
    let world = economy.apply_planning_to_world(state);
    let mut from_buildings = 0.0;
    for building in world
        .buildings
        .iter()
        .filter(|b| b.type_script_id(&economy.defs) == BUILDING_CONSTRUCTION_SECTOR)
    {
        let level = building.level.max(0.0);
        if level <= 0.0 {
            continue;
        }
        let add = expect_construction_add(building, &economy.defs);
        from_buildings += add * level;
    }
    let points_per_day = base + from_buildings;
    if points_per_day.is_finite() && points_per_day > 0.0 {
        points_per_day
    } else {
        f64::from(config.base_construction_capacity.max(1))
    }
}

/// Government construction throughput (**points/day**) from CS levels and laws.
///
/// `government = national × government_share(laws)`. Non-finite or non-positive
/// results fall back to at least one point of base throughput so progress never
/// stalls.
pub fn construction_points_per_day_from_sectors(
    state: &PlanningState,
    economy: &EconomyContext,
    config: SimConfig,
) -> f64 {
    let national = national_construction_points_per_day(state, economy, config);
    let share = government_construction_share_from_laws(state.laws.iter().map(String::as_str));
    let govt = national * share;
    if govt.is_finite() && govt > 0.0 {
        govt
    } else {
        f64::from(config.base_construction_capacity.max(1))
    }
}

/// Refresh [`PlanningState::construction_points_per_day`] from current CS levels.
///
/// Call after a Construction Sector level completes so subsequent waits use the
/// higher throughput. Uses [`construction_points_per_day_from_sectors`] with the
/// live economy world (government share applied).
pub fn sync_construction_points_per_day(
    state: &mut PlanningState,
    economy: &EconomyContext,
    config: SimConfig,
) {
    state.construction_points_per_day =
        construction_points_per_day_from_sectors(state, economy, config);
}

/// Max weekly construction progress (points/week) from base + owned techs.
///
/// Approximation table from vanilla society techs; company / other modifiers
/// are omitted. Convert to a daily per-job cap with `/ 7`.
pub fn max_weekly_construction_progress(state: &PlanningState) -> f64 {
    let mut weekly = BASE_MAX_WEEKLY_CONSTRUCTION_PROGRESS;
    for tech in &state.techs {
        weekly += max_weekly_progress_add_for_tech(tech);
    }
    if weekly.is_finite() && weekly > 0.0 {
        weekly
    } else {
        BASE_MAX_WEEKLY_CONSTRUCTION_PROGRESS
    }
}

fn max_weekly_progress_add_for_tech(tech: &str) -> f64 {
    match tech {
        "urbanization" => 10.0,
        "urban_planning" | "modern_sewerage" | "steel_frame_buildings" | "elevator" => 5.0,
        _ => 0.0,
    }
}

/// Per-job allocation cap in **points/day**.
///
/// Prefer [`SimConfig::max_construction_allocation`] when set (tests / escape
/// hatch). Otherwise `max_weekly_construction_progress(state) / 7`.
///
/// `building` is reserved for future building-group construction-efficiency
/// scaling; currently unused (efficiency approx = 1).
pub fn allocation_cap_points_per_day(
    state: &PlanningState,
    config: SimConfig,
    _building: Option<&str>,
) -> f64 {
    if let Some(override_cap) = config.max_construction_allocation {
        return f64::from(override_cap.max(1));
    }
    let daily = max_weekly_construction_progress(state) / 7.0;
    if daily.is_finite() && daily > 0.0 {
        daily
    } else {
        BASE_MAX_WEEKLY_CONSTRUCTION_PROGRESS / 7.0
    }
}

/// How many government construction jobs may receive points under current throughput.
///
/// `max(1, floor(points_per_day / allocation_cap_points_per_day))` as
/// [`ConstructionSlots`]. A zero or non-finite national throughput still yields
/// [`ConstructionSlots::ONE`] so one government job remains **admissible to
/// enqueue**; allocation may still feed 0 points/day until throughput is
/// positive (see [`construction_points_per_day_per_job`]).
pub fn max_parallel_construction_jobs(
    construction_points_per_day: f64,
    max_points_per_day_per_job: f64,
) -> ConstructionSlots {
    let cap = if max_points_per_day_per_job.is_finite() && max_points_per_day_per_job > 0.0 {
        max_points_per_day_per_job
    } else {
        1.0
    };
    if !construction_points_per_day.is_finite() || construction_points_per_day <= 0.0 {
        return ConstructionSlots::ONE;
    }
    ConstructionSlots::new((construction_points_per_day / cap).floor() as usize)
}

fn government_job_count(state: &PlanningState) -> usize {
    state
        .constructions
        .iter()
        .filter(|job| job.queue == ConstructionQueueKind::Government)
        .count()
}

/// True when compact parallel **government** construction slots are full.
///
/// [`crate::sim::Action::QueueBuildingLevel`] is rejected while this holds so
/// the search does not enqueue deeper than government throughput can actively
/// feed. Private queue rows do not consume government slots.
pub fn construction_queue_full(state: &PlanningState, config: SimConfig) -> bool {
    let cap = allocation_cap_points_per_day(state, config, None);
    let max = max_parallel_construction_jobs(state.construction_points_per_day, cap);
    government_job_count(state) >= max.get()
}

/// Per-job construction throughput (**points/day**) from the government pool.
///
/// Walks [`PlanningState::constructions`] in order. Private rows get `0.0`.
/// Each of the first [`max_parallel_construction_jobs`] **government** entries
/// receives `min(allocation_cap, remaining_pool)` until the pool is exhausted.
/// Later government entries get `0.0` (queued but idle).
///
/// The returned slice length always matches `state.constructions.len()` so it
/// can be passed to [`PlanningState::tick_parallel_tracks`].
pub fn construction_points_per_day_per_job(state: &PlanningState, config: SimConfig) -> Vec<f64> {
    let government = state.construction_points_per_day;
    let mut remaining = if government.is_finite() && government > 0.0 {
        government
    } else {
        0.0
    };
    let default_cap = allocation_cap_points_per_day(state, config, None);
    let max_jobs = max_parallel_construction_jobs(government, default_cap).get();
    let mut out = vec![0.0; state.constructions.len()];
    let mut fed = 0usize;
    for (idx, job) in state.constructions.iter().enumerate() {
        if job.queue != ConstructionQueueKind::Government {
            continue;
        }
        if fed >= max_jobs || remaining <= 0.0 {
            break;
        }
        let cap = allocation_cap_points_per_day(state, config, Some(job.building.as_str()));
        let take = cap.min(remaining);
        out[idx] = take;
        remaining -= take;
        fed += 1;
    }
    out
}

/// Leftover government points/day after feeding active jobs (for capacity ETA).
pub fn unused_government_construction_points_per_day(
    state: &PlanningState,
    config: SimConfig,
) -> f64 {
    let feeds = construction_points_per_day_per_job(state, config);
    let used: f64 = feeds.iter().copied().sum();
    let pool = state.construction_points_per_day;
    if !pool.is_finite() || pool <= 0.0 {
        return 0.0;
    }
    (pool - used).max(0.0)
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

/// Construction ETA used by the A* heuristic (and tests).
///
/// See [`ConstructionEtaMode`] for capacity-vs-next-finish semantics. The old
/// blanket `.max(1)` is not applied when the bound is a multi-day capacity
/// estimate; a zero-day pending completion still reports `0` so a 0-day
/// `BuildingCompleted` edge stays consistent.
pub fn construction_eta_days(
    state: &PlanningState,
    config: SimConfig,
    mode: ConstructionEtaMode,
) -> u32 {
    let fallback = u32::from(config.construction_days.max(1));
    match mode {
        ConstructionEtaMode::NextFinish => construction_wait_days(state, config)
            .map(u32::from)
            .unwrap_or(fallback),
        ConstructionEtaMode::CapacityOrSlot => {
            if construction_queue_full(state, config) {
                // Slot-bound: wait until a fed government job finishes.
                return construction_wait_days(state, config)
                    .map(u32::from)
                    .unwrap_or(fallback)
                    .max(1);
            }
            // Spare government slot / leftover feed: lower-bound by one level
            // at the unused (or full) government rate — not next-finish.
            let rate = {
                let unused = unused_government_construction_points_per_day(state, config);
                if unused > 0.0 {
                    unused.min(allocation_cap_points_per_day(state, config, None))
                } else if state.construction_points_per_day.is_finite()
                    && state.construction_points_per_day > 0.0
                {
                    allocation_cap_points_per_day(state, config, None)
                        .min(state.construction_points_per_day)
                } else {
                    0.0
                }
            };
            let work = f64::from(config.default_construction_cost);
            crate::tracks::days_for_work(work, rate)
                .unwrap_or(fallback)
                .max(1)
        }
    }
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
        .building_types
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
    use vic3_defs::{BuildingType, GameDefs, GoodsVec, ProductionMethod};
    use vic3_prices::{SolveOpts, WorldBuilding, WorldCountry, WorldState};

    #[test]
    fn government_share_from_economic_laws() {
        assert!(
            (government_construction_share_from_laws(["law_laissez_faire"]) - 0.25).abs() < 1e-9
        );
        assert!(
            (government_construction_share_from_laws(["law_command_economy"]) - 0.9).abs() < 1e-9
        );
        assert!(
            (government_construction_share_from_laws(["law_traditionalism"]) - 0.75).abs() < 1e-9
        );
        assert!(
            (government_construction_share_from_laws(["law_census_voting"]) - 1.0).abs() < 1e-9
        );
    }

    #[test]
    fn cs_building_requires_country_construction_add_pm() {
        let mut defs = GameDefs::default();
        defs.production_methods.insert(
            "pm_not_construction".into(),
            ProductionMethod {
                id: "pm_not_construction".into(),
                ..ProductionMethod::default()
            },
        );
        defs.building_types.insert(
            BUILDING_CONSTRUCTION_SECTOR.into(),
            BuildingType {
                id: BUILDING_CONSTRUCTION_SECTOR.into(),
                group: None,
                city_type: None,
                production_method_groups: Vec::new(),
                required_construction: Some(10.0),
            },
        );
        defs.building_types_order
            .push(BUILDING_CONSTRUCTION_SECTOR.into());
        let building = WorldBuilding {
            id: 7,
            state: Some(1),
            building_type_id: defs
                .building_index_of(BUILDING_CONSTRUCTION_SECTOR)
                .expect("cs"),
            level: 1.0,
            staffing: 1.0,
            production_methods: vec!["pm_not_construction".into()],
            saved_inputs: Vec::new(),
            saved_outputs: Vec::new(),
        };
        let err = construction_add_for_cs_building(&building, &defs).unwrap_err();
        assert_eq!(err.building_id, 7);
        assert_eq!(err.methods, ["pm_not_construction"]);
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
        let config = SimConfig {
            max_construction_allocation: Some(1000),
            ..SimConfig::default()
        };
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
            max_construction_allocation: Some(5),
            ..SimConfig::default()
        };
        assert_eq!(max_parallel_construction_jobs(10.0, 5.0).get(), 2);
        assert_eq!(
            max_parallel_construction_jobs(0.0, 5.0),
            ConstructionSlots::ONE
        );
        assert_eq!(
            max_parallel_construction_jobs(f64::NAN, 5.0),
            ConstructionSlots::ONE
        );
        let feeds = construction_points_per_day_per_job(&state, config);
        assert_eq!(feeds, vec![5.0, 5.0]);
        assert_eq!(construction_wait_days(&state, config), Some(10));
    }

    #[test]
    fn private_jobs_do_not_consume_government_pool() {
        let state = PlanningState::from_parts(PlanningParts {
            constructions: vec![
                PlanningConstruction {
                    order_id: 1,
                    queue: ConstructionQueueKind::Private,
                    state_id: None,
                    building: "building_private".into(),
                    remaining: Some(50.0),
                },
                PlanningConstruction {
                    order_id: 2,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_govt".into(),
                    remaining: Some(50.0),
                },
            ],
            construction_points_per_day: 10.0,
            ..PlanningParts::default()
        });
        let config = SimConfig {
            max_construction_allocation: Some(10),
            ..SimConfig::default()
        };
        let feeds = construction_points_per_day_per_job(&state, config);
        assert_eq!(feeds, vec![0.0, 10.0]);
        // One government job with pool 10 / cap 10 → slots full of government work only.
        assert!(construction_queue_full(&state, config));
        let roomy = PlanningState::from_parts(PlanningParts {
            constructions: state.constructions.clone(),
            construction_points_per_day: 20.0,
            ..PlanningParts::default()
        });
        assert!(!construction_queue_full(&roomy, config));
        assert_eq!(
            construction_points_per_day_per_job(&roomy, config),
            vec![0.0, 10.0]
        );
    }

    #[test]
    fn derived_alloc_cap_uses_weekly_progress_over_seven() {
        let state = PlanningState::default();
        let config = SimConfig::default();
        let cap = allocation_cap_points_per_day(&state, config, None);
        assert!((cap - BASE_MAX_WEEKLY_CONSTRUCTION_PROGRESS / 7.0).abs() < 1e-9);
        let with_tech = PlanningState::from_parts(PlanningParts {
            techs: ["urbanization".into()].into_iter().collect(),
            ..PlanningParts::default()
        });
        let cap_tech = allocation_cap_points_per_day(&with_tech, config, None);
        assert!((cap_tech - 20.0 / 7.0).abs() < 1e-9);
    }

    #[test]
    fn capacity_eta_uses_one_level_when_slots_free() {
        let state = PlanningState::from_parts(PlanningParts {
            construction_points_per_day: 10.0,
            ..PlanningParts::default()
        });
        let config = SimConfig {
            max_construction_allocation: Some(5),
            default_construction_cost: 50,
            ..SimConfig::default()
        };
        // Spare slots, empty queue: ceil(50 / min(5,10)) = 10.
        assert_eq!(
            construction_eta_days(&state, config, ConstructionEtaMode::CapacityOrSlot),
            10
        );
    }

    #[test]
    fn capacity_eta_uses_next_finish_when_slots_full() {
        let state = PlanningState::from_parts(PlanningParts {
            constructions: vec![
                PlanningConstruction {
                    order_id: 1,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_a".into(),
                    remaining: Some(20.0),
                },
                PlanningConstruction {
                    order_id: 2,
                    queue: ConstructionQueueKind::Government,
                    state_id: None,
                    building: "building_b".into(),
                    remaining: Some(100.0),
                },
            ],
            construction_points_per_day: 10.0,
            ..PlanningParts::default()
        });
        let config = SimConfig {
            max_construction_allocation: Some(5),
            default_construction_cost: 180,
            ..SimConfig::default()
        };
        assert!(construction_queue_full(&state, config));
        // Next finish: 20/5 = 4 days (not default_cost/rate).
        assert_eq!(
            construction_eta_days(&state, config, ConstructionEtaMode::CapacityOrSlot),
            4
        );
        assert_eq!(
            construction_eta_days(&state, config, ConstructionEtaMode::NextFinish),
            4
        );
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
    fn sync_points_per_day_after_cs_level_applies_government_share() {
        let mut defs = GameDefs::default();
        defs.building_types.insert(
            BUILDING_CONSTRUCTION_SECTOR.into(),
            BuildingType {
                id: BUILDING_CONSTRUCTION_SECTOR.into(),
                group: None,
                city_type: None,
                production_method_groups: Vec::new(),
                required_construction: Some(10.0),
            },
        );
        defs.building_types_order
            .push(BUILDING_CONSTRUCTION_SECTOR.into());
        defs.production_methods.insert(
            "pm_iron_frame_buildings".into(),
            ProductionMethod {
                id: "pm_iron_frame_buildings".into(),
                country_construction_add: Some(5.0),
                ..ProductionMethod::default()
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
                building_type_id: defs
                    .building_index_of(BUILDING_CONSTRUCTION_SECTOR)
                    .expect("cs"),
                level: 0.0,
                staffing: 0.0,
                production_methods: vec!["pm_iron_frame_buildings".into()],
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            }],
            frozen_buy: GoodsVec::from_vec(vec![]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let config = SimConfig {
            base_construction_capacity: 1,
            ..SimConfig::default()
        };
        let mut state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            construction_points_per_day: 1.0,
            building_level_deltas: BTreeMap::from([((BUILDING_CONSTRUCTION_SECTOR.into(), 1), 1)]),
            // laissez-faire → 25% government
            laws: ["law_laissez_faire".into()].into_iter().collect(),
            ..PlanningParts::default()
        });
        sync_construction_points_per_day(&mut state, &economy, config);
        // CS level 1 → national 1 + 5 = 6; government 25% → 1.5
        assert!((state.construction_points_per_day - 1.5).abs() < 1e-9);
    }
}
