//! Compact [`PlanningState`] — the planning stack's world node.
//!
//! # Role
//!
//! Downstream crates never search the full save IR. They read this projection:
//! - `crate::goals` evaluates DSL atoms against it
//! - `crate::sim` emits goal-relevant successors that mutate a clone
//! - `crate::plan` interns A* nodes by [`PlanningState::fingerprint`] (I8)
//!
//! This is **not** the full save. Identical state ⇒ identical hash (I8).
//!
//! # Fill: save vs default
//!
//! | Field | At load (`from_save` / `from_world`) | Default / sim-only |
//! | --- | --- | --- |
//! | `date`, `country` | save meta / tag | `1836.1.1` / empty |
//! | `techs`, `queued_tech`, `queued_building`, `constructions` | tech manager + construction queues | empty |
//! | `laws`, `infamy` | law manager / country | empty / `None` |
//! | `good_prices` | last price solve | empty map |
//! | `gdp` | `0` unless `*_with_prices` (owned building revenue) | `0` |
//! | budget / `solvent` / SoL proxy | country budget + owned-state pops | false / `0` / `None` |
//! | army / navy / interest | country cache + formations; PP `None` when IR omits | `None` / empty |
//! | `building_level_deltas`, `pm_overrides`, `tax_level` | empty / `0` | sim branches |
//! | `queued_interest`, `queued_hire`, `queued_law`, `mil_buildings` | empty / `None` | sim in-flight queues |
//! | `construction_points_per_day` | Government construction points/day (CS × govt share) | sim / tests |
//! | `tech_days_left` / `interest_days_left` / `hire_days_left` / `law_days_left` | `None` at load | set on queue / decremented by [`PlanningState::tick_parallel_tracks`] |
//!
//! Missing principal/credit leaves `solvent` false (no treasury-sign guess).
//! Unknown country tag → [`WorldError::UnknownCountry`].
//!
//! See [`docs/planning.md`](../../../docs/planning.md).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use vic3_defs::GameDefs;
use vic3_prices::{PricesResult, World, WorldCountry};

use crate::construction::{
    construction_points_per_day_from_save, construction_points_per_day_from_world,
};
use crate::military::{recompute_army_pp, recompute_navy_pp, ModeledMilBuilding, UnitCombatStats};

pub use vic3_load::{ConstructionQueueKind, Save, Vic3Date};

/// Limitation when save IR has no army power projection (not a measured zero).
pub const ARMY_POWER_PROJECTION_UNKNOWN: &str =
    "army power projection unknown in save IR (not a measured zero)";

/// Limitation when save IR has no navy power projection (not a measured zero).
pub const NAVY_POWER_PROJECTION_UNKNOWN: &str =
    "navy power projection unknown in save IR (not a measured zero)";

/// Limitation when military buildings on the plan path are underemployed.
pub const MILITARY_UNDEREMPLOYED: &str =
    "barracks / shipyards / naval administrations must be fully staffed for power projection";

/// Failure while projecting a [`PlanningState`] from save IR or [`World`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorldError {
    /// No live country whose `definition` / tag matches the requested tag.
    #[error("country tag `{0}` not found in save")]
    UnknownCountry(String),
    /// Construction Sector building lacks a required defs PM with
    /// `country_construction_add`.
    #[error(transparent)]
    ConstructionPm(#[from] crate::construction::MissingConstructionSectorPm),
}

/// Game start used when a save has no `meta_data.game_date`.
pub fn default_date() -> Vic3Date {
    Vic3Date::from_ymdh(1836, 1, 1, 0)
}

/// Compact interest declaration in flight on a planning branch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum QueuedInterest {
    /// DSL `interest_in(state=…)`.
    State(String),
    /// DSL `interest_in(region=…)`.
    Region(String),
}

/// One construction-queue row on a planning branch (player country only).
///
/// Full ordered queue for exposure / SQL consumers. Distinct from
/// [`PlanningState::queued_building`], which remains the single in-flight head
/// used by sim waits (`QueueBuildingLevel` / `BuildingCompleted`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningConstruction {
    pub order_id: u32,
    pub queue: ConstructionQueueKind,
    pub state_id: Option<u32>,
    pub building: String,
    pub remaining: Option<f64>,
}

impl PartialEq for PlanningConstruction {
    fn eq(&self, other: &Self) -> bool {
        self.order_id == other.order_id
            && self.queue == other.queue
            && self.state_id == other.state_id
            && self.building == other.building
            && f64_option_bits_eq(self.remaining, other.remaining)
    }
}

impl Eq for PlanningConstruction {}

impl Hash for PlanningConstruction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.order_id.hash(state);
        self.queue.hash(state);
        self.state_id.hash(state);
        self.building.hash(state);
        hash_f64_option(self.remaining, state);
    }
}

impl From<&vic3_load::ConstructionQueueEntry> for PlanningConstruction {
    fn from(entry: &vic3_load::ConstructionQueueEntry) -> Self {
        Self {
            order_id: entry.order_id,
            queue: entry.queue,
            state_id: entry.state_id,
            building: entry.building.clone(),
            remaining: entry.remaining,
        }
    }
}

impl From<&vic3_prices::WorldConstruction> for PlanningConstruction {
    fn from(entry: &vic3_prices::WorldConstruction) -> Self {
        Self {
            order_id: entry.id,
            queue: match entry.queue {
                vic3_prices::ConstructionQueueKind::Private => ConstructionQueueKind::Private,
                vic3_prices::ConstructionQueueKind::Government => ConstructionQueueKind::Government,
            },
            state_id: entry.state_id,
            building: entry.building.clone(),
            remaining: entry.remaining,
        }
    }
}

/// Inputs for [`PlanningState::from_parts`] (tests; no `.v3` required).
#[derive(Debug, Clone)]
pub struct PlanningParts {
    pub date: Vic3Date,
    pub country: String,
    pub techs: Vec<String>,
    pub good_prices: Vec<(String, f64)>,
    pub solvent: bool,
    pub treasury: f64,
    /// Known army power projection; `None` when save IR omits it (not zero).
    pub army_power_projection: Option<f64>,
    /// Known navy power projection; `None` when save IR omits it (not zero).
    pub navy_power_projection: Option<f64>,
    /// Save baseline army PP before sim-added mil buildings.
    pub army_pp_baseline: Option<f64>,
    /// Save baseline navy PP before sim-added mil buildings.
    pub navy_pp_baseline: Option<f64>,
    /// State ids for DSL `interest_in(state=…)`.
    pub interest: Vec<String>,
    /// Region ids for DSL `interest_in(region=…)`.
    pub interest_regions: Vec<String>,
    pub queued_tech: Option<String>,
    pub gdp: f64,
    pub weekly_balance: Option<f64>,
    pub population_weighted_wealth: Option<f64>,
    pub debt_principal: Option<f64>,
    pub credit_limit: Option<f64>,
    pub credit_headroom: Option<f64>,
    /// `(building_type, state_id)` → added levels on this branch.
    pub building_level_deltas: BTreeMap<(String, u32), u32>,
    pub queued_building: Option<String>,
    /// Full private then government queue for this country (exposure / sim sync).
    pub constructions: Vec<PlanningConstruction>,
    pub queued_interest: Option<QueuedInterest>,
    /// Building type currently hiring toward full employment (sim branch).
    pub queued_hire: Option<String>,
    /// Sim-added barracks / shipyards / naval administrations.
    pub mil_buildings: Vec<ModeledMilBuilding>,
    /// Active law script ids (`law_autocracy`, …).
    pub laws: Vec<String>,
    /// Law currently being enacted (sim branch only).
    pub queued_law: Option<String>,
    /// Country infamy when present on the save.
    pub infamy: Option<f64>,
    /// Building-id → production methods overridden on this branch.
    ///
    /// **Required and must be valid** when used: each id must resolve against
    /// game defs (same contract as [`vic3_prices::WorldBuilding::production_methods`]).
    /// Open design question: string script ids vs indices into a bidirectional
    /// name↔id map (not remapped in this crate yet).
    pub pm_overrides: BTreeMap<u32, Vec<String>>,
    /// Tax level offset from the saved baseline (`0` at load).
    pub tax_level: i8,
    /// Construction points completed per day (default 1.0).
    pub construction_points_per_day: f64,
    /// Days left on the research queue head (`None` when idle).
    pub tech_days_left: Option<u16>,
    /// Days left on the interest queue head.
    pub interest_days_left: Option<u16>,
    /// Days left on the hire queue head.
    pub hire_days_left: Option<u16>,
    /// Days left on the law queue head.
    pub law_days_left: Option<u16>,
}

impl Default for PlanningParts {
    fn default() -> Self {
        Self {
            date: default_date(),
            country: String::new(),
            techs: Vec::new(),
            good_prices: Vec::new(),
            solvent: false,
            treasury: 0.0,
            army_power_projection: None,
            navy_power_projection: None,
            army_pp_baseline: None,
            navy_pp_baseline: None,
            interest: Vec::new(),
            interest_regions: Vec::new(),
            queued_tech: None,
            gdp: 0.0,
            weekly_balance: None,
            population_weighted_wealth: None,
            debt_principal: None,
            credit_limit: None,
            credit_headroom: None,
            building_level_deltas: BTreeMap::new(),
            queued_building: None,
            constructions: Vec::new(),
            queued_interest: None,
            queued_hire: None,
            mil_buildings: Vec::new(),
            laws: Vec::new(),
            queued_law: None,
            infamy: None,
            pm_overrides: BTreeMap::new(),
            tax_level: 0,
            construction_points_per_day: 1.0,
            tech_days_left: None,
            interest_days_left: None,
            hire_days_left: None,
            law_days_left: None,
        }
    }
}

/// Compact world node for DSL eval, sim successors, and A* intern keys.
///
/// Fields cover every simple subgoal `crate::goals` can read, plus queue / delta slots
/// `crate::sim` needs for waits and re-solves. Hash/eq use `f64::to_bits` (I8)
/// for discrete floats that are part of identity. **`good_prices` and `gdp` are
/// omitted** from Hash/Eq by default (derived solve outputs); set
/// `VIC3_PLAN_FP_INCLUDE_PRICES=1` to restore the old include-for-A/B traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningState {
    pub date: Vic3Date,
    /// Country tag (`GER`, `FRA`, …).
    pub country: String,
    /// Researched technology ids (save + completed sim research).
    pub techs: BTreeSet<String>,
    /// Market prices after the last solve (good id → price).
    pub good_prices: BTreeMap<String, f64>,
    /// True only when known `credit_headroom > 0` (not treasury sign).
    pub solvent: bool,
    pub treasury: f64,
    /// Army power projection when save IR exposes it; `None` is unknown (not zero).
    pub army_power_projection: Option<f64>,
    /// Navy power projection when save IR exposes it; `None` is unknown (not zero).
    pub navy_power_projection: Option<f64>,
    /// Baseline army PP from save (before sim-added mil buildings).
    pub army_pp_baseline: Option<f64>,
    /// Baseline navy PP from save (before sim-added mil buildings).
    pub navy_pp_baseline: Option<f64>,
    /// Normalized state ids for DSL `interest_in(state=…)`.
    pub interest_states: BTreeSet<String>,
    /// Normalized strategic-region ids for DSL `interest_in(region=…)`.
    pub interest_regions: BTreeSet<String>,
    /// Research queue head from save, or sim `QueueTech`.
    pub queued_tech: Option<String>,
    /// Modeled GDP (`*_with_prices` / sim re-solve); else `0` at load.
    pub gdp: f64,
    /// Most recent finite saved net weekly-budget sample.
    pub weekly_balance: Option<f64>,
    /// Population-weighted saved pop wealth (SoL proxy); `None` if unknown.
    pub population_weighted_wealth: Option<f64>,
    /// Outstanding debt principal when present on the save budget.
    pub debt_principal: Option<f64>,
    /// Credit limit when present on the save budget.
    pub credit_limit: Option<f64>,
    /// `credit_limit - debt_principal` when both are known.
    pub credit_headroom: Option<f64>,
    /// Added levels by `(building_type, state_id)` on this branch (empty at load).
    ///
    /// Matches Vic3 construction placement: a level is always queued in a state.
    pub building_level_deltas: BTreeMap<(String, u32), u32>,
    /// Construction queue head from save, or sim `QueueBuildingLevel`.
    ///
    /// Prefer private over government at load. Kept in sync with
    /// [`Self::constructions`] when sim queues or completes a building.
    pub queued_building: Option<String>,
    /// Full ordered construction queue for this country (private then government).
    ///
    /// Exposed for SQL / UI / future goals. Sim push/pops entries alongside
    /// [`Self::queued_building`] so the list does not diverge from the head.
    pub constructions: Vec<PlanningConstruction>,
    /// Interest in flight — sim-only (`None` at load).
    pub queued_interest: Option<QueuedInterest>,
    /// Hire-to-full in flight for a mil building type — sim-only.
    pub queued_hire: Option<String>,
    /// Sim-added barracks / shipyards / naval administrations.
    pub mil_buildings: Vec<ModeledMilBuilding>,
    /// Active law script ids from save / completed enactments.
    pub laws: BTreeSet<String>,
    /// Law enactment in flight — sim-only (`None` at load).
    pub queued_law: Option<String>,
    /// Country infamy when present (`None` when missing); not yet a DSL simple subgoal.
    pub infamy: Option<f64>,
    /// Per-building PM overrides on this branch (empty at load).
    ///
    /// **Required and must be valid** when non-empty: each method id must resolve
    /// against game defs. Same string-id vs bidirectional index-map design
    /// question as [`vic3_prices::WorldBuilding::production_methods`].
    pub pm_overrides: BTreeMap<u32, Vec<String>>,
    /// Tax offset from saved baseline (`0` at load; sim `AdjustTax`).
    pub tax_level: i8,
    /// National **government** construction throughput in **construction points per day**.
    ///
    /// Derived from Construction Sector levels × **required** CS PM
    /// `country_construction_add` (defs), scaled by the government share of the
    /// national pool (economic laws). There is no geo-state allocation of that
    /// pool. Private queue rows do not draw from this throughput. Used with
    /// queue `remaining` (points) so ETA is `ceil(remaining / points_per_day)`
    /// when a government job is fed (or less under the per-job allocation cap).
    pub construction_points_per_day: f64,
    /// Remaining days on [`Self::queued_tech`] (model timer).
    pub tech_days_left: Option<u16>,
    /// Remaining days on [`Self::queued_interest`].
    pub interest_days_left: Option<u16>,
    /// Remaining days on [`Self::queued_hire`].
    pub hire_days_left: Option<u16>,
    /// Remaining days on [`Self::queued_law`].
    pub law_days_left: Option<u16>,
}

impl Default for PlanningState {
    fn default() -> Self {
        Self::from_parts(PlanningParts::default())
    }
}

impl PartialEq for PlanningState {
    fn eq(&self, other: &Self) -> bool {
        self.date == other.date
            && self.country == other.country
            && self.techs == other.techs
            && (!hash_includes_derived_prices()
                || (f64_map_eq(&self.good_prices, &other.good_prices)
                    && f64_bits_eq(self.gdp, other.gdp)))
            && self.solvent == other.solvent
            && f64_bits_eq(self.treasury, other.treasury)
            && f64_option_bits_eq(self.army_power_projection, other.army_power_projection)
            && f64_option_bits_eq(self.navy_power_projection, other.navy_power_projection)
            && f64_option_bits_eq(self.army_pp_baseline, other.army_pp_baseline)
            && f64_option_bits_eq(self.navy_pp_baseline, other.navy_pp_baseline)
            && self.interest_states == other.interest_states
            && self.interest_regions == other.interest_regions
            && self.queued_tech == other.queued_tech
            && f64_option_bits_eq(self.weekly_balance, other.weekly_balance)
            && f64_option_bits_eq(
                self.population_weighted_wealth,
                other.population_weighted_wealth,
            )
            && f64_option_bits_eq(self.debt_principal, other.debt_principal)
            && f64_option_bits_eq(self.credit_limit, other.credit_limit)
            && f64_option_bits_eq(self.credit_headroom, other.credit_headroom)
            && self.building_level_deltas == other.building_level_deltas
            && self.queued_building == other.queued_building
            && self.constructions == other.constructions
            && self.queued_interest == other.queued_interest
            && self.queued_hire == other.queued_hire
            && self.mil_buildings == other.mil_buildings
            && self.laws == other.laws
            && self.queued_law == other.queued_law
            && f64_option_bits_eq(self.infamy, other.infamy)
            && self.pm_overrides == other.pm_overrides
            && self.tax_level == other.tax_level
            && f64_bits_eq(
                self.construction_points_per_day,
                other.construction_points_per_day,
            )
            && self.tech_days_left == other.tech_days_left
            && self.interest_days_left == other.interest_days_left
            && self.hire_days_left == other.hire_days_left
            && self.law_days_left == other.law_days_left
    }
}

impl Eq for PlanningState {}

impl Hash for PlanningState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.date.hash(state);
        self.country.hash(state);
        self.techs.hash(state);
        if hash_includes_derived_prices() {
            self.good_prices.len().hash(state);
            for (k, v) in &self.good_prices {
                k.hash(state);
                v.to_bits().hash(state);
            }
            self.gdp.to_bits().hash(state);
        }
        self.solvent.hash(state);
        self.treasury.to_bits().hash(state);
        hash_f64_option(self.army_power_projection, state);
        hash_f64_option(self.navy_power_projection, state);
        hash_f64_option(self.army_pp_baseline, state);
        hash_f64_option(self.navy_pp_baseline, state);
        self.interest_states.hash(state);
        self.interest_regions.hash(state);
        self.queued_tech.hash(state);
        hash_f64_option(self.weekly_balance, state);
        hash_f64_option(self.population_weighted_wealth, state);
        hash_f64_option(self.debt_principal, state);
        hash_f64_option(self.credit_limit, state);
        hash_f64_option(self.credit_headroom, state);
        self.building_level_deltas.hash(state);
        self.queued_building.hash(state);
        self.constructions.hash(state);
        self.queued_interest.hash(state);
        self.queued_hire.hash(state);
        self.mil_buildings.hash(state);
        self.laws.hash(state);
        self.queued_law.hash(state);
        hash_f64_option(self.infamy, state);
        self.pm_overrides.len().hash(state);
        for (id, methods) in &self.pm_overrides {
            id.hash(state);
            methods.hash(state);
        }
        self.tax_level.hash(state);
        self.construction_points_per_day.to_bits().hash(state);
        self.tech_days_left.hash(state);
        self.interest_days_left.hash(state);
        self.hire_days_left.hash(state);
        self.law_days_left.hash(state);
    }
}

/// When true, `good_prices` / `gdp` participate in Hash/Eq (legacy A* identity).
///
/// Default false (omit derived solve floats). Set `VIC3_PLAN_FP_INCLUDE_PRICES=1`
/// for before/after duplicate-rate traces against the old fingerprint.
fn hash_includes_derived_prices() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| match std::env::var("VIC3_PLAN_FP_INCLUDE_PRICES") {
        Ok(v) => {
            let t = v.trim();
            !(t.is_empty() || t == "0" || t.eq_ignore_ascii_case("false"))
        }
        Err(_) => false,
    })
}

fn f64_bits_eq(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

fn f64_option_bits_eq(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => f64_bits_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn hash_f64_option<H: Hasher>(value: Option<f64>, state: &mut H) {
    value.is_some().hash(state);
    if let Some(value) = value {
        value.to_bits().hash(state);
    }
}

fn f64_map_eq(a: &BTreeMap<String, f64>, b: &BTreeMap<String, f64>) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && f64_bits_eq(*va, *vb))
}

/// Something that can supply good id → price for [`PlanningState::from_save`]
/// / [`PlanningState::from_world`].
pub trait IntoPriceMap {
    fn into_price_map(self) -> BTreeMap<String, f64>;
}

impl IntoPriceMap for &PricesResult {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self.goods
            .iter()
            .map(|g| (g.name.clone(), g.price))
            .collect()
    }
}

impl IntoPriceMap for PricesResult {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        (&self).into_price_map()
    }
}

impl IntoPriceMap for BTreeMap<String, f64> {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self
    }
}

impl IntoPriceMap for &BTreeMap<String, f64> {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self.clone()
    }
}

impl IntoPriceMap for Vec<(String, f64)> {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self.into_iter().collect()
    }
}

impl PlanningState {
    /// Fake projection for tests. Does not load a `.v3`.
    pub fn from_parts(parts: PlanningParts) -> Self {
        Self {
            date: parts.date,
            country: parts.country,
            techs: parts.techs.into_iter().collect(),
            good_prices: parts.good_prices.into_iter().collect(),
            solvent: parts.solvent,
            treasury: parts.treasury,
            army_power_projection: parts.army_power_projection,
            navy_power_projection: parts.navy_power_projection,
            army_pp_baseline: parts.army_pp_baseline.or(parts.army_power_projection),
            navy_pp_baseline: parts.navy_pp_baseline.or(parts.navy_power_projection),
            interest_states: parts.interest.into_iter().collect(),
            interest_regions: parts.interest_regions.into_iter().collect(),
            queued_tech: parts.queued_tech,
            gdp: parts.gdp,
            weekly_balance: parts.weekly_balance,
            population_weighted_wealth: parts.population_weighted_wealth,
            debt_principal: parts.debt_principal,
            credit_limit: parts.credit_limit,
            credit_headroom: parts.credit_headroom,
            building_level_deltas: parts.building_level_deltas,
            queued_building: parts.queued_building,
            constructions: parts.constructions,
            queued_interest: parts.queued_interest,
            queued_hire: parts.queued_hire,
            mil_buildings: parts.mil_buildings,
            laws: parts.laws.into_iter().collect(),
            queued_law: parts.queued_law,
            infamy: parts.infamy,
            pm_overrides: parts.pm_overrides,
            tax_level: parts.tax_level,
            construction_points_per_day: if parts.construction_points_per_day.is_finite()
                && parts.construction_points_per_day > 0.0
            {
                parts.construction_points_per_day
            } else {
                1.0
            },
            tech_days_left: parts.tech_days_left,
            interest_days_left: parts.interest_days_left,
            hire_days_left: parts.hire_days_left,
            law_days_left: parts.law_days_left,
        }
    }

    /// Whether any track currently has an in-flight job (any queue non-empty).
    pub fn has_inflight_queue(&self) -> bool {
        self.research_busy()
            || self.construction_busy()
            || self.interest_busy()
            || self.hire_busy()
            || self.law_busy()
    }

    /// Research track has a queued tech.
    pub fn research_busy(&self) -> bool {
        self.queued_tech.is_some()
    }

    /// Construction backlog is non-empty.
    pub fn construction_busy(&self) -> bool {
        self.queued_building.is_some() || !self.constructions.is_empty()
    }

    /// Interest track busy.
    pub fn interest_busy(&self) -> bool {
        self.queued_interest.is_some()
    }

    /// Hire track busy.
    pub fn hire_busy(&self) -> bool {
        self.queued_hire.is_some()
    }

    /// Law track busy.
    pub fn law_busy(&self) -> bool {
        self.queued_law.is_some()
    }

    /// Advance parallel track timers by `days` without completing the waited event.
    ///
    /// Each construction job loses `days * alloc[i]` work when `construction_alloc`
    /// provides a per-index rate (capacity split). When the slice is shorter than
    /// the queue, missing entries receive zero. Fixed-day tracks decrement their
    /// `*_days_left` counters (saturating at 0). A zero-day tick is a no-op.
    pub fn tick_parallel_tracks(&mut self, days: u16, construction_alloc: &[f64]) {
        if days == 0 {
            return;
        }
        if let Some(left) = self.tech_days_left.as_mut() {
            *left = left.saturating_sub(days);
        }
        if let Some(left) = self.interest_days_left.as_mut() {
            *left = left.saturating_sub(days);
        }
        if let Some(left) = self.hire_days_left.as_mut() {
            *left = left.saturating_sub(days);
        }
        if let Some(left) = self.law_days_left.as_mut() {
            *left = left.saturating_sub(days);
        }
        for (job, rate) in self.constructions.iter_mut().zip(construction_alloc.iter()) {
            if !rate.is_finite() || *rate <= 0.0 {
                continue;
            }
            if let Some(rem) = job.remaining.as_mut() {
                if rem.is_finite() {
                    *rem = (*rem - f64::from(days) * *rate).max(0.0);
                }
            }
        }
    }

    /// Refresh army/navy PP from baselines + staffed mil buildings.
    pub fn recompute_military_pp(&mut self) {
        self.army_power_projection = recompute_army_pp(
            self.army_pp_baseline,
            &self.mil_buildings,
            UnitCombatStats::army_default(),
        );
        self.navy_power_projection = recompute_navy_pp(
            self.navy_pp_baseline,
            &self.mil_buildings,
            UnitCombatStats::navy_default(),
        );
    }

    /// True when sim-added mil buildings are fully staffed (or none exist).
    pub fn mil_buildings_fully_staffed(&self) -> bool {
        crate::military::military_buildings_fully_staffed(&self.mil_buildings)
    }

    pub fn army_buildings_fully_staffed(&self) -> bool {
        crate::military::army_buildings_fully_staffed(&self.mil_buildings)
    }

    pub fn navy_buildings_fully_staffed(&self) -> bool {
        crate::military::navy_buildings_fully_staffed(&self.mil_buildings)
    }

    /// Add one underemployed level of a military building type.
    pub fn push_mil_building_level(&mut self, building: &str) {
        if let Some(row) = self
            .mil_buildings
            .iter_mut()
            .find(|row| row.building == building)
        {
            row.levels += 1.0;
        } else {
            self.mil_buildings.push(ModeledMilBuilding {
                building: building.to_string(),
                levels: 1.0,
                staffing: 0.0,
            });
        }
        self.recompute_military_pp();
    }

    /// Raise staffing for `building` up to its levels (full hire).
    pub fn complete_mil_hire(&mut self, building: &str) {
        if let Some(row) = self
            .mil_buildings
            .iter_mut()
            .find(|row| row.building == building)
        {
            row.staffing = row.levels;
        }
        self.recompute_military_pp();
    }

    /// Limitation line when army power projection is missing from save IR.
    pub fn army_power_unknown_limitation(&self) -> Option<&'static str> {
        self.army_power_projection
            .is_none()
            .then_some(ARMY_POWER_PROJECTION_UNKNOWN)
    }

    /// Limitation line when navy power projection is missing from save IR.
    pub fn navy_power_unknown_limitation(&self) -> Option<&'static str> {
        self.navy_power_projection
            .is_none()
            .then_some(NAVY_POWER_PROJECTION_UNKNOWN)
    }

    /// Append [`ARMY_POWER_PROJECTION_UNKNOWN`] when projection is missing.
    pub fn push_army_power_limitation(&self, limitations: &mut Vec<String>) {
        if let Some(line) = self.army_power_unknown_limitation() {
            if !limitations.iter().any(|existing| existing == line) {
                limitations.push(line.to_string());
            }
        }
    }

    /// Append [`NAVY_POWER_PROJECTION_UNKNOWN`] when projection is missing.
    pub fn push_navy_power_limitation(&self, limitations: &mut Vec<String>) {
        if let Some(line) = self.navy_power_unknown_limitation() {
            if !limitations.iter().any(|existing| existing == line) {
                limitations.push(line.to_string());
            }
        }
    }

    /// Append underemployment limitation when mil buildings are not fully staffed.
    pub fn push_military_staffing_limitation(&self, limitations: &mut Vec<String>) {
        if !self.mil_buildings_fully_staffed()
            && !limitations
                .iter()
                .any(|existing| existing == MILITARY_UNDEREMPLOYED)
        {
            limitations.push(MILITARY_UNDEREMPLOYED.to_string());
        }
    }

    /// Rebuild [`Self::queued_building`] from the first [`Self::constructions`] entry.
    pub fn sync_queued_building_from_constructions(&mut self) {
        self.queued_building = self
            .constructions
            .first()
            .map(|entry| entry.building.clone());
    }

    /// Append a government construction (sim `QueueBuildingLevel`).
    ///
    /// `remaining` is construction work points when known (typically
    /// [`vic3_defs::BuildingType::required_construction`] for a new enqueue).
    /// Pass [`None`] when the def omits cost so wait ETA can fall back to
    /// [`crate::sim::SimConfig::construction_days`]. In-flight save rows keep
    /// their own `remaining` and are not rewritten here. The in-flight head
    /// stays the first queue entry (`sync_queued_building_from_constructions`).
    ///
    /// `state_id` is the Vic3 placement state (required for planner enqueues).
    pub fn push_construction(&mut self, building: String, state_id: u32, remaining: Option<f64>) {
        let order_id = self
            .constructions
            .iter()
            .map(|entry| entry.order_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.constructions.push(PlanningConstruction {
            order_id,
            queue: ConstructionQueueKind::Government,
            state_id: Some(state_id),
            building,
            remaining,
        });
        self.sync_queued_building_from_constructions();
    }

    /// Remove a completed construction entry matching `building` (+ optional state), then advance the head.
    ///
    /// Prefers a finished row (`remaining <= 0`) when several share the type so
    /// parallel completions pop the right job. When `state_id` is [`Some`], only
    /// that placement matches; [`None`] matches any state (save rows).
    pub fn complete_construction(
        &mut self,
        defs: &vic3_defs::GameDefs,
        building: &str,
        state_id: Option<u32>,
    ) {
        let matches = |entry: &PlanningConstruction| {
            defs.building_types_equivalent(&entry.building, building)
                && state_id
                    .map(|want| entry.state_id == Some(want) || entry.state_id.is_none())
                    .unwrap_or(true)
        };
        let finished = self.constructions.iter().position(|entry| {
            matches(entry)
                && entry
                    .remaining
                    .is_some_and(|rem| rem.is_finite() && rem <= 0.0)
        });
        let idx = finished.or_else(|| self.constructions.iter().position(matches));
        if let Some(idx) = idx {
            self.constructions.remove(idx);
        }
        self.sync_queued_building_from_constructions();
    }

    /// Project the player country and last price solve into a planning node.
    ///
    /// Researched techs and research/construction queue heads come from save IR
    /// (`technology` manager, country tech fields, construction queues). Army
    /// power prefers country `cached_total_army_power_projection`, else army
    /// formation `power_projection`. Interest comes from
    /// `interest_marker_manager` / country `declared_interests`. Weekly balance
    /// is the last saved sample; population-weighted wealth is computed from
    /// pops in states owned by this country. Solvency requires known principal
    /// and credit. Active laws and infamy come from country / law manager rows.
    /// Missing metrics remain `None`. GDP defaults to `0` here; use
    /// [`Self::from_save_with_prices`] for modeled GDP.
    ///
    /// `defs` is required so Construction Sector throughput can resolve
    /// `country_construction_add` from building PMs (no silent per-level guess).
    pub fn from_save(
        save: &Save,
        country_tag: &str,
        prices: impl IntoPriceMap,
        defs: &GameDefs,
    ) -> Result<Self, WorldError> {
        let (country_id, country) = save
            .countries()
            .find(|(_, country)| country.definition == country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let treasury = country.budget.treasury().unwrap_or(0.0);
        let weekly_balance = country
            .budget
            .weekly_income
            .last()
            .copied()
            .filter(|value| value.is_finite());
        let debt_principal = country.budget.principal.filter(|value| value.is_finite());
        let credit_limit = country.budget.credit.filter(|value| value.is_finite());
        let credit_headroom = country.budget.credit_headroom();
        let mut owned_states: BTreeSet<u32> = save
            .states
            .iter_present()
            .filter_map(|(id, state)| (state.country == Some(country_id)).then_some(id))
            .collect();
        if owned_states.is_empty() {
            owned_states.extend(country.states.iter().copied());
        }
        let (weighted_sol, population) = save
            .pops
            .iter_present()
            .filter_map(|(_, pop)| {
                let state = pop.state?;
                if !owned_states.contains(&state) {
                    return None;
                }
                let size = pop.demand_size()?;
                let wealth = f64::from(pop.wealth?);
                Some((wealth * size, size))
            })
            .fold(
                (0.0, 0.0),
                |(wealth, population), (next_wealth, next_population)| {
                    (wealth + next_wealth, population + next_population)
                },
            );
        let population_weighted_wealth = if population > 0.0 {
            Some(weighted_sol / population)
        } else {
            None
        };
        let interest = save.declared_interest_for(country_id);
        Ok(Self {
            date: save.meta_data.game_date.unwrap_or_else(default_date),
            country: country.definition.clone(),
            techs: save.researched_techs_for(country_id).into_iter().collect(),
            good_prices: prices.into_price_map(),
            solvent: country.budget.is_solvent(),
            treasury,
            army_power_projection: save.army_power_projection_for(country_id),
            navy_power_projection: save.navy_power_projection_for(country_id),
            army_pp_baseline: save.army_power_projection_for(country_id),
            navy_pp_baseline: save.navy_power_projection_for(country_id),
            interest_states: interest.states.into_iter().collect(),
            interest_regions: interest.regions.into_iter().collect(),
            queued_tech: save.queued_tech_for(country_id),
            gdp: 0.0,
            weekly_balance,
            population_weighted_wealth,
            debt_principal,
            credit_limit,
            credit_headroom,
            building_level_deltas: BTreeMap::new(),
            queued_building: save.queued_building_for(country_id),
            constructions: vic3_load::constructions_for(save, country_id)
                .iter()
                .map(PlanningConstruction::from)
                .collect(),
            // Interest/hire/law queues and PM/tax deltas are sim-only.
            queued_interest: None,
            queued_hire: None,
            mil_buildings: Vec::new(),
            laws: save
                .active_laws(country_id)
                .into_iter()
                .map(str::to_string)
                .collect(),
            queued_law: None,
            infamy: country.infamy.filter(|value| value.is_finite()),
            pm_overrides: BTreeMap::new(),
            tax_level: 0,
            construction_points_per_day: construction_points_per_day_from_save(
                save, country_id, defs,
            )?,
            tech_days_left: None,
            interest_days_left: None,
            hire_days_left: None,
            law_days_left: None,
        })
    }

    /// Project from a full price result, including modeled GDP.
    ///
    /// GDP is gross building output value (`revenue`) in states owned by the
    /// selected country under the same solved prices.
    pub fn from_save_with_prices(
        save: &Save,
        country_tag: &str,
        prices: &PricesResult,
        defs: &GameDefs,
    ) -> Result<Self, WorldError> {
        let (country_id, country) = save
            .countries()
            .find(|(_, country)| country.definition == country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let mut owned_states: BTreeSet<u32> = save
            .states
            .iter_present()
            .filter_map(|(id, state)| (state.country == Some(country_id)).then_some(id))
            .collect();
        if owned_states.is_empty() {
            owned_states.extend(country.states.iter().copied());
        }
        let mut state = Self::from_save(save, country_tag, prices, defs)?;
        state.gdp = prices
            .buildings
            .iter()
            .filter(|building| {
                building
                    .state_id
                    .is_some_and(|state_id| owned_states.contains(&state_id))
            })
            .map(|building| building.revenue.max(0.0))
            .sum();
        Ok(state)
    }

    /// Project the player country from a compact [`World`] plus last prices.
    ///
    /// Techs and research/construction queue heads come from [`WorldCountry`]
    /// (filled by [`World::from_save`] from save IR). Army power and declared
    /// interest are projected the same way. Laws and infamy come from
    /// [`WorldCountry`]. Population-weighted wealth uses pops in states owned
    /// by this country. Solvency and budget lines come from [`WorldCountry`].
    /// GDP defaults to `0`; use [`Self::from_world_with_prices`] for modeled GDP.
    ///
    /// `defs` resolves Construction Sector `country_construction_add` PMs.
    pub fn from_world(
        world: &World,
        country_tag: &str,
        prices: impl IntoPriceMap,
        defs: &GameDefs,
    ) -> Result<Self, WorldError> {
        let country = world
            .country_by_tag(country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        Ok(Self {
            date: world.game_date.unwrap_or_else(default_date),
            country: country.tag.clone(),
            techs: country.techs.iter().cloned().collect(),
            good_prices: prices.into_price_map(),
            solvent: country.solvent,
            treasury: country.treasury,
            army_power_projection: country.army_power_projection,
            navy_power_projection: country.navy_power_projection,
            army_pp_baseline: country.army_power_projection,
            navy_pp_baseline: country.navy_power_projection,
            interest_states: country.interest_states.iter().cloned().collect(),
            interest_regions: country.interest_regions.iter().cloned().collect(),
            queued_tech: country.queued_tech.clone(),
            gdp: 0.0,
            weekly_balance: country.weekly_balance,
            population_weighted_wealth: population_weighted_wealth(world, country),
            debt_principal: country.debt_principal,
            credit_limit: country.credit_limit,
            credit_headroom: country.credit_headroom,
            building_level_deltas: BTreeMap::new(),
            queued_building: country.queued_building.clone(),
            constructions: world
                .constructions
                .iter()
                .filter(|row| row.country_id == Some(country.id))
                .map(PlanningConstruction::from)
                .collect(),
            queued_interest: None,
            queued_hire: None,
            mil_buildings: Vec::new(),
            laws: country.laws.iter().cloned().collect(),
            queued_law: None,
            infamy: country.infamy,
            pm_overrides: BTreeMap::new(),
            tax_level: 0,
            construction_points_per_day: construction_points_per_day_from_world(
                world, country.id, defs,
            )?,
            tech_days_left: None,
            interest_days_left: None,
            hire_days_left: None,
            law_days_left: None,
        })
    }

    /// Project from a full price result, including modeled GDP.
    ///
    /// GDP is gross building output value (`revenue`) in states owned by the
    /// selected country under the same solved prices.
    pub fn from_world_with_prices(
        world: &World,
        country_tag: &str,
        prices: &PricesResult,
        defs: &GameDefs,
    ) -> Result<Self, WorldError> {
        let country = world
            .country_by_tag(country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let owned_states = owned_state_ids(world, country);
        let mut state = Self::from_world(world, country_tag, prices, defs)?;
        state.gdp = prices
            .buildings
            .iter()
            .filter(|building| {
                building
                    .state_id
                    .is_some_and(|state_id| owned_states.contains(&state_id))
            })
            .map(|building| building.revenue.max(0.0))
            .sum();
        Ok(state)
    }

    /// I8 fingerprint: identical state ⇒ identical `u64`.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    /// Price of `good` after the last solve, if present.
    pub fn price(&self, good: &str) -> Option<f64> {
        self.good_prices.get(good).copied()
    }

    /// Whether `tech` is in the researched set.
    pub fn has_tech(&self, tech: &str) -> bool {
        self.techs.contains(tech)
    }

    /// Whether the country has interest matching DSL `state=` / `region=` ids.
    pub fn has_interest(&self, id: &str) -> bool {
        self.interest_states.contains(id) || self.interest_regions.contains(id)
    }

    /// Whether `interest_in(state=id)` holds.
    pub fn has_interest_state(&self, id: &str) -> bool {
        self.interest_states.contains(id)
    }

    /// Whether `interest_in(region=id)` holds.
    pub fn has_interest_region(&self, id: &str) -> bool {
        self.interest_regions.contains(id)
    }

    /// Whether `has_law(id)` holds (case / `law_` prefix insensitive).
    pub fn has_law(&self, id: &str) -> bool {
        let key = law_key(id);
        self.laws.iter().any(|law| law_key(law) == key)
    }
}

/// Compact law id for DSL matching (`law_autocracy` ↔ `autocracy`).
pub fn law_key(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    lower
        .strip_prefix("law_")
        .unwrap_or(lower.as_str())
        .to_string()
}

fn owned_state_ids(world: &World, country: &WorldCountry) -> BTreeSet<u32> {
    let mut owned_states: BTreeSet<u32> = world
        .states
        .iter()
        .filter_map(|state| (state.country == Some(country.id)).then_some(state.id))
        .collect();
    if owned_states.is_empty() {
        owned_states.extend(country.states.iter().copied());
    }
    owned_states
}

fn population_weighted_wealth(world: &World, country: &WorldCountry) -> Option<f64> {
    let owned_states = owned_state_ids(world, country);
    let (weighted_sol, population) = world
        .iter_pops()
        .filter_map(|pop| {
            let state = pop.state?;
            if !owned_states.contains(&state) {
                return None;
            }
            Some((f64::from(pop.wealth) * pop.size, pop.size))
        })
        .fold(
            (0.0, 0.0),
            |(wealth, population), (next_wealth, next_population)| {
                (wealth + next_wealth, population + next_population)
            },
        );
    (population > 0.0).then_some(weighted_sol / population)
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vic3_load::{Budget, Country, Manager, Meta, Pop, Save, State};
    use vic3_prices::{BuildingEconomics, GoodPrice, PricesResult, SolveStatus, World};

    fn ger_save(treasury: f64) -> Save {
        ger_budget_save(
            treasury,
            Budget {
                gold_reserves: Some(treasury),
                credit: Some(500.0),
                principal: Some(0.0),
                weekly_income: vec![50.0, 100.0],
                ..Budget::default()
            },
        )
    }

    fn ger_budget_save(treasury: f64, budget: Budget) -> Save {
        let mut countries = Manager::<Country>::default();
        countries.database.insert(
            1,
            Some(Country {
                definition: "GER".into(),
                budget: Budget {
                    gold_reserves: budget.gold_reserves.or(Some(treasury)),
                    ..budget
                },
                ..Country::default()
            }),
        );
        Save {
            meta_data: Meta {
                version: "1.9.0".into(),
                game_date: Some(Vic3Date::from_ymdh(1850, 6, 1, 0)),
                name: Some("Fixture".into()),
            },
            country_manager: countries,
            ..Save::default()
        }
    }

    fn ammo_prices(price: f64) -> PricesResult {
        PricesResult {
            scope: "whole_save_synthetic".into(),
            goods: vec![GoodPrice {
                name: "ammunition".into(),
                label: None,
                base: 50.0,
                price,
                buy: 1.0,
                sell: 1.0,
            }],
            states: Vec::new(),
            state_goods: Vec::new(),
            buildings: Vec::new(),
            building_types: Vec::new(),
            building_groups: Vec::new(),
            state_pops: Vec::new().into(),
            countries: Vec::new(),
            inputs: Default::default(),
            residual: 0.0,
            status: SolveStatus::Converged,
            limitations: Vec::new(),
            state_qualifications: Vec::new(),
            state_needs: Vec::new(),
            relative: Vec::new(),
        }
    }

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn from_parts_fake_state_no_save() {
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["nitroglycerin".into()],
            good_prices: vec![("ammunition".into(), 32.0)],
            solvent: true,
            treasury: 1_000.0,
            army_power_projection: Some(120.0),
            interest: vec!["alsace".into()],
            queued_tech: Some("mechanized_agriculture".into()),
            gdp: 50e6,
            ..PlanningParts::default()
        });
        assert_eq!(state.country, "GER");
        assert!(state.has_tech("nitroglycerin"));
        assert_eq!(state.price("ammunition"), Some(32.0));
        assert!(state.solvent);
        assert!(state.has_interest("alsace"));
        assert_eq!(state.queued_tech.as_deref(), Some("mechanized_agriculture"));
        assert_eq!(state.army_power_projection, Some(120.0));
        assert_eq!(state.gdp, 50e6);
    }

    #[test]
    fn from_save_uses_prices_result_and_treasury() {
        let save = ger_save(10_000.0);
        let state = PlanningState::from_save(&save, "GER", ammo_prices(40.0), &GameDefs::default())
            .unwrap();
        assert_eq!(state.country, "GER");
        assert_eq!(state.date, Vic3Date::from_ymdh(1850, 6, 1, 0));
        assert_eq!(state.treasury, 10_000.0);
        assert!(state.solvent);
        assert_eq!(state.weekly_balance, Some(100.0));
        assert_eq!(state.debt_principal, Some(0.0));
        assert_eq!(state.credit_limit, Some(500.0));
        assert_eq!(state.credit_headroom, Some(500.0));
        assert_eq!(state.price("ammunition"), Some(40.0));
        assert_eq!(state.army_power_projection, None);
        assert_eq!(
            state.army_power_unknown_limitation(),
            Some(ARMY_POWER_PROJECTION_UNKNOWN)
        );
        assert!(state.techs.is_empty());
        assert!(state.interest_states.is_empty());
        assert!(state.interest_regions.is_empty());
        assert!(state.queued_tech.is_none());
        assert!(state.queued_building.is_none());
    }

    #[test]
    fn from_save_projects_techs_and_queue_heads() {
        let mut save = ger_save(10_000.0);
        save.country_manager
            .database
            .get_mut(&1)
            .unwrap()
            .as_mut()
            .unwrap()
            .techs = vec!["railways".into()];
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.technology.database.insert(
            1,
            Some(vic3_load::TechnologyEntry {
                country: Some(1),
                acquired_technologies: vec!["nitroglycerin".into(), "urban_planning".into()],
                research_technology: Some("atmospheric_engine".into()),
            }),
        );
        save.building_constructions.database.insert(
            1,
            Some(vic3_load::ConstructionOrder {
                building: Some("building_construction_sector".into()),
                state: Some(10),
                remaining: Some(20.0),
            }),
        );

        let state =
            PlanningState::from_save(&save, "GER", BTreeMap::new(), &GameDefs::default()).unwrap();
        assert!(state.has_tech("railways"));
        assert!(state.has_tech("nitroglycerin"));
        assert!(state.has_tech("urban_planning"));
        assert_eq!(state.queued_tech.as_deref(), Some("atmospheric_engine"));
        assert_eq!(
            state.queued_building.as_deref(),
            Some("building_construction_sector")
        );
        assert_eq!(state.constructions.len(), 1);
        assert_eq!(
            state.constructions[0].building,
            "building_construction_sector"
        );

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let from_world =
            PlanningState::from_world(&world, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert_eq!(from_world.techs, state.techs);
        assert_eq!(from_world.queued_tech, state.queued_tech);
        assert_eq!(from_world.queued_building, state.queued_building);
        assert_eq!(from_world.constructions, state.constructions);
    }

    #[test]
    fn fixture_save_projects_techs_and_queues() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-load/tests/fixtures/plaintext.txt");
        let save = vic3_load::load_path(&path, vic3_load::empty_tokens()).expect("fixture");
        let state =
            PlanningState::from_save(&save, "GER", BTreeMap::new(), &GameDefs::default()).unwrap();
        assert!(state.has_tech("urban_planning"));
        assert!(state.has_tech("railways"));
        assert!(!state.has_tech("mechanized_farming"));
        assert!(!state.has_tech("nitroglycerin"));
        assert!(state.queued_tech.is_none());
        assert!(state.queued_building.is_none());
        assert!(state.has_law("law_autocracy") || state.has_law("autocracy"));
        assert_eq!(state.infamy, Some(12.5));

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let from_world =
            PlanningState::from_world(&world, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert_eq!(from_world, state);
        assert_eq!(world.countries[0].infamy, Some(12.5));
    }

    #[test]
    fn from_save_solvent_uses_credit_headroom_not_treasury_sign() {
        let indebted = ger_budget_save(
            -50.0,
            Budget {
                gold_reserves: Some(-50.0),
                credit: Some(1_000.0),
                principal: Some(200.0),
                weekly_income: vec![10.0],
                ..Budget::default()
            },
        );
        let indebted_state =
            PlanningState::from_save(&indebted, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert!(indebted_state.solvent);
        assert_eq!(indebted_state.credit_headroom, Some(800.0));

        let exhausted = ger_budget_save(
            1_000.0,
            Budget {
                gold_reserves: Some(1_000.0),
                credit: Some(500.0),
                principal: Some(500.0),
                ..Budget::default()
            },
        );
        let exhausted_state =
            PlanningState::from_save(&exhausted, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert!(!exhausted_state.solvent);
        assert_eq!(exhausted_state.credit_headroom, Some(0.0));

        let unknown = ger_budget_save(
            1_000.0,
            Budget {
                gold_reserves: Some(1_000.0),
                ..Budget::default()
            },
        );
        let unknown_state =
            PlanningState::from_save(&unknown, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert!(!unknown_state.solvent);
        assert_eq!(unknown_state.credit_headroom, None);
    }

    #[test]
    fn from_save_population_weights_wealth_for_owned_states() {
        let mut save = ger_save(10_000.0);
        save.country_manager
            .database
            .get_mut(&1)
            .unwrap()
            .as_mut()
            .unwrap()
            .states = vec![20];
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        save.pops.database.insert(
            1,
            Some(Pop {
                size: Some(1_000.0),
                wealth: Some(10),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            2,
            Some(Pop {
                size: Some(3_000.0),
                wealth: Some(20),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            3,
            Some(Pop {
                size: Some(10_000.0),
                wealth: Some(99),
                state: Some(20),
                ..Pop::default()
            }),
        );

        let state =
            PlanningState::from_save(&save, "GER", BTreeMap::new(), &GameDefs::default()).unwrap();
        assert_eq!(state.population_weighted_wealth, Some(17.5));
    }

    #[test]
    fn from_save_with_prices_models_gdp_from_owned_building_revenue() {
        let mut save = ger_save(10_000.0);
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        let mut prices = ammo_prices(40.0);
        let building = |id, state_id, revenue| BuildingEconomics {
            id,
            state_id: Some(state_id),
            building_type_id: None,
            building_type_name: "building_factory".into(),
            building_type_label: None,
            level: 1.0,
            staffing: 1.0,
            production_method_ids: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            revenue,
            cost: 0.0,
            profit: revenue,
            short_inputs: Vec::new(),
            employees: Vec::new(),
        };
        prices.buildings = vec![building(1, 10, 125.0), building(2, 20, 900.0)];

        let state =
            PlanningState::from_save_with_prices(&save, "GER", &prices, &GameDefs::default())
                .unwrap();
        assert_eq!(state.gdp, 125.0);
    }

    #[test]
    fn from_save_accepts_price_map() {
        let save = ger_save(-50.0);
        let mut prices = BTreeMap::new();
        prices.insert("grain".into(), 20.0);
        let state = PlanningState::from_save(&save, "GER", prices, &GameDefs::default()).unwrap();
        assert!(state.solvent);
        assert_eq!(state.treasury, -50.0);
        assert_eq!(state.credit_headroom, Some(500.0));
        assert_eq!(state.price("grain"), Some(20.0));
        assert_eq!(state.population_weighted_wealth, None);
    }

    #[test]
    fn from_save_unknown_tag() {
        let save = ger_save(1.0);
        let err = PlanningState::from_save(&save, "FRA", BTreeMap::new(), &GameDefs::default())
            .unwrap_err();
        assert_eq!(err, WorldError::UnknownCountry("FRA".into()));
    }

    #[test]
    fn from_world_matches_from_save_budget_and_date() {
        let save = ger_save(10_000.0);
        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let from_save =
            PlanningState::from_save(&save, "GER", ammo_prices(40.0), &GameDefs::default())
                .unwrap();
        let from_world =
            PlanningState::from_world(&world, "GER", ammo_prices(40.0), &GameDefs::default())
                .unwrap();
        assert_eq!(from_save, from_world);
        assert_eq!(world.player_country_tag(), Some("GER"));
        assert_eq!(world.game_date, Some(Vic3Date::from_ymdh(1850, 6, 1, 0)));
    }

    #[test]
    fn from_world_population_weights_wealth_for_owned_states() {
        let mut save = ger_save(10_000.0);
        save.country_manager
            .database
            .get_mut(&1)
            .unwrap()
            .as_mut()
            .unwrap()
            .states = vec![20];
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        save.pops.database.insert(
            1,
            Some(Pop {
                size: Some(1_000.0),
                wealth: Some(10),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            2,
            Some(Pop {
                size: Some(3_000.0),
                wealth: Some(20),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            3,
            Some(Pop {
                size: Some(10_000.0),
                wealth: Some(99),
                state: Some(20),
                ..Pop::default()
            }),
        );

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let state = PlanningState::from_world(&world, "GER", BTreeMap::new(), &GameDefs::default())
            .unwrap();
        assert_eq!(state.population_weighted_wealth, Some(17.5));
    }

    #[test]
    fn from_world_with_prices_models_gdp_from_owned_building_revenue() {
        let mut save = ger_save(10_000.0);
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        let mut prices = ammo_prices(40.0);
        let building = |id, state_id, revenue| BuildingEconomics {
            id,
            state_id: Some(state_id),
            building_type_id: None,
            building_type_name: "building_factory".into(),
            building_type_label: None,
            level: 1.0,
            staffing: 1.0,
            production_method_ids: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            revenue,
            cost: 0.0,
            profit: revenue,
            short_inputs: Vec::new(),
            employees: Vec::new(),
        };
        prices.buildings = vec![building(1, 10, 125.0), building(2, 20, 900.0)];

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let state =
            PlanningState::from_world_with_prices(&world, "GER", &prices, &GameDefs::default())
                .unwrap();
        assert_eq!(state.gdp, 125.0);
    }

    #[test]
    fn from_world_unknown_tag() {
        let save = ger_save(1.0);
        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let err = PlanningState::from_world(&world, "FRA", BTreeMap::new(), &GameDefs::default())
            .unwrap_err();
        assert_eq!(err, WorldError::UnknownCountry("FRA".into()));
    }

    #[test]
    fn i8_identical_planning_state_identical_hash() {
        let a = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["railways".into(), "nitroglycerin".into()],
            good_prices: vec![("wood".into(), 20.0), ("grain".into(), 30.0)],
            solvent: true,
            treasury: 4.0,
            army_power_projection: Some(10.0),
            interest: vec!["alsace".into()],
            gdp: 1.0,
            ..PlanningParts::default()
        });
        let b = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["nitroglycerin".into(), "railways".into()],
            good_prices: vec![("grain".into(), 30.0), ("wood".into(), 20.0)],
            solvent: true,
            treasury: 4.0,
            army_power_projection: Some(10.0),
            interest: vec!["alsace".into()],
            gdp: 1.0,
            ..PlanningParts::default()
        });
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint(), a.clone().fingerprint());

        let mut other = a.clone();
        other.treasury = 5.0;
        assert_ne!(a, other);
        assert_ne!(a.fingerprint(), other.fingerprint());
    }

    #[test]
    fn fingerprint_ignores_good_prices_and_gdp_by_default() {
        let a = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            building_level_deltas: [(("building_rye_farm".into(), 1), 1)].into_iter().collect(),
            good_prices: vec![("grain".into(), 20.0)],
            gdp: 1.0e6,
            ..PlanningParts::default()
        });
        let mut b = a.clone();
        b.good_prices.insert("grain".into(), 99.0);
        b.gdp = 2.0e6;
        assert_eq!(
            a.fingerprint(),
            b.fingerprint(),
            "derived prices/gdp must not split A* identity"
        );
        assert_eq!(a, b);
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// I8 for this struct: identical [`PlanningState`] ⇒ identical hash.
        #[test]
        fn i8_clone_round_trip_same_hash(
            country in "[A-Z]{3}",
            treasury in -1_000.0f64..1_000.0,
            army in 0.0f64..500.0,
            solvent in any::<bool>(),
            gdp in 0.0f64..1e9,
            ammo in 1.0f64..100.0,
        ) {
            let state = PlanningState::from_parts(PlanningParts {
                country,
                treasury,
                army_power_projection: Some(army),
                solvent,
                gdp,
                good_prices: vec![("ammunition".into(), ammo)],
                ..PlanningParts::default()
            });
            let clone = state.clone();
            prop_assert_eq!(&state, &clone);
            prop_assert_eq!(state.fingerprint(), clone.fingerprint());
        }
    }

    #[test]
    fn from_save_projects_army_power_and_interest() {
        let mut save = ger_save(10_000.0);
        {
            let country = save
                .country_manager
                .database
                .get_mut(&1)
                .unwrap()
                .as_mut()
                .unwrap();
            country.cached_total_army_power_projection = Some(210.0);
            country.declared_interests = vec!["region_north_africa".into()];
        }
        save.interest_marker_manager.database.insert(
            1,
            Some(vic3_load::InterestMarker {
                country: Some(1),
                strategic_region: Some("sr:region_western_europe".into()),
                state: Some("STATE_ALSACE".into()),
                kind: None,
            }),
        );

        let state =
            PlanningState::from_save(&save, "GER", BTreeMap::new(), &GameDefs::default()).unwrap();
        assert_eq!(state.army_power_projection, Some(210.0));
        assert!(state.has_interest_state("alsace"));
        assert!(!state.has_interest_region("alsace"));
        assert!(state.has_interest_region("region_western_europe"));
        assert!(state.has_interest_region("region_north_africa"));
        assert!(!state.has_interest_state("region_western_europe"));

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let from_world =
            PlanningState::from_world(&world, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert_eq!(from_world.army_power_projection, Some(210.0));
        assert!(from_world.has_interest_state("alsace"));
        assert!(from_world.has_interest_region("region_western_europe"));
    }

    #[test]
    fn from_save_army_power_unknown_not_silent_zero() {
        let save = ger_save(10_000.0);
        let state =
            PlanningState::from_save(&save, "GER", BTreeMap::new(), &GameDefs::default()).unwrap();
        assert_eq!(state.army_power_projection, None);
        assert_eq!(
            state.army_power_unknown_limitation(),
            Some(ARMY_POWER_PROJECTION_UNKNOWN)
        );
        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let from_world =
            PlanningState::from_world(&world, "GER", BTreeMap::new(), &GameDefs::default())
                .unwrap();
        assert_eq!(from_world.army_power_projection, None);
        assert_eq!(
            from_world.army_power_unknown_limitation(),
            Some(ARMY_POWER_PROJECTION_UNKNOWN)
        );
    }
}
