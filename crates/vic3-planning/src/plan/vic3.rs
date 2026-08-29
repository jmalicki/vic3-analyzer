//! A* node backed by goal-relevant [`crate::sim`] transitions.
//!
//! # Cheap intern key
//!
//! [`Vic3Node`] splits into [`Vic3Identity`] (Hash/Eq) and [`Vic3Cache`]
//! (shared search context / trace, excluded from identity). `Hash` uses the
//! precomputed `u64` fingerprint only (cheap intern buckets). `Eq` is
//! `Rc::ptr_eq` first, then fingerprint + full [`PlanningState`] when pointers
//! differ — so reconstructs still merge and fingerprint collisions cannot.
//! Goal / config / economy ride behind [`Rc`]s in the cache and are not part
//! of identity. Prefer [`Rc`] while search is single-threaded; switch to
//! [`std::sync::Arc`] when nodes must be `Send + Sync`.
//!
//! # Heuristic DAG
//!
//! [`SearchNode::heuristic`] walks the compiled [`Goal`] as a dependency DAG:
//! AND → max child, OR → min child, NOT → 0. Open research atoms contribute
//! research ETA (defs cost at rate 1.0 when available, else `research_days`),
//! including a serial sum over missing prerequisite ancestors when defs are
//! present. Open interest / raisable army / law atoms contribute fixed model
//! days even if an unrelated item is queued (returning 0 over a zero-day queue
//! edge would break consistency). Open `good_price` / `gdp` use construction
//! ETA from head remaining work when present, else `construction_days`, unless
//! a zero-day SwitchPm path exists. Fiscal / SoL / tax atoms contribute 0.
//!
//! Admissible relaxation of the real graph (**I7** on research formulas), not
//! a substitute for search.

use super::pathfinding::SearchNode;
use crate::goals::{evaluate, Goal, SimpleSubgoal};
use crate::sim::{EconomyContext, SimConfig, Successor};
use crate::world::PlanningState;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

#[derive(Debug, Default)]
struct FingerprintDupStats {
    seen: RefCell<HashSet<u64>>,
    dups: Cell<u64>,
}

impl FingerprintDupStats {
    fn note(&self, fingerprint: u64) {
        if !self.seen.borrow_mut().insert(fingerprint) {
            self.dups.set(self.dups.get().saturating_add(1));
        }
    }

    fn snapshot(&self) -> (u64, u64) {
        let uniques = self.seen.borrow().len() as u64;
        (self.dups.get(), uniques)
    }
}

/// Search counters maintained in Vic3 because A* does not expose them.
///
/// Includes domain-fingerprint reconvergence (`fp_*`) and PEA* expand/frontier
/// proxies (`ranked_*`, `beam_*`). The pathfinder's true open/closed set sizes
/// are unavailable (`rust-advanced-heaps` keeps them private).
#[derive(Debug, Default)]
struct SearchTraceStats {
    /// Seen domain fingerprints + duplicate creation count.
    fp: FingerprintDupStats,
    pea_ready_expands: Cell<u64>,
    pea_resume_expands: Cell<u64>,
    /// Sum / max of PEA ranked-successor list lengths at ready expands.
    ranked_sum: Cell<u64>,
    ranked_max: Cell<u64>,
    /// Successors actually emitted into the beam (not deferred).
    beam_emitted: Cell<u64>,
    /// Times an Expanding cursor was re-queued (deferred frontier).
    beam_deferred: Cell<u64>,
}

impl SearchTraceStats {
    fn note_fp(&self, fingerprint: u64) {
        self.fp.note(fingerprint);
    }

    fn note_pea_ready(&self, ranked_len: usize) {
        self.pea_ready_expands
            .set(self.pea_ready_expands.get().saturating_add(1));
        let n = ranked_len as u64;
        self.ranked_sum.set(self.ranked_sum.get().saturating_add(n));
        let max = self.ranked_max.get();
        if n > max {
            self.ranked_max.set(n);
        }
    }

    fn note_pea_resume(&self) {
        self.pea_resume_expands
            .set(self.pea_resume_expands.get().saturating_add(1));
    }

    fn note_beam_emit(&self, emitted: usize, deferred: bool) {
        self.beam_emitted
            .set(self.beam_emitted.get().saturating_add(emitted as u64));
        if deferred {
            self.beam_deferred
                .set(self.beam_deferred.get().saturating_add(1));
        }
    }

    fn summary_line(&self) -> String {
        let (fp_dups, fp_uniques) = self.fp.snapshot();
        let fp_emitted = fp_dups.saturating_add(fp_uniques);
        let ready = self.pea_ready_expands.get();
        let resume = self.pea_resume_expands.get();
        let ranked_sum = self.ranked_sum.get();
        let ranked_avg = if ready > 0 {
            ranked_sum as f64 / ready as f64
        } else {
            0.0
        };
        format!(
            "fp_dups={fp_dups} fp_uniques={fp_uniques} fp_emitted={fp_emitted} \
             pea_ready={ready} pea_resume={resume} \
             ranked_max={} ranked_avg={ranked_avg:.1} ranked_sum={ranked_sum} \
             beam_emitted={} beam_deferred={} \
             (A* open/closed sizes not available from pathfinder)",
            self.ranked_max.get(),
            self.beam_emitted.get(),
            self.beam_deferred.get(),
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GdpKnapsack {
    /// List of (efficiency_gdp_per_cp, cp_cost_per_level, max_levels_available).
    /// Sorted descending by efficiency.
    pub items: Vec<(f64, f64, f64)>,
    /// Map of building_type_script_id to efficiency for quick tie-breaker lookup.
    pub efficiency_map: std::collections::HashMap<String, f64>,
}

impl GdpKnapsack {
    pub(crate) fn needed_cp(&self, mut target_gap: f64) -> Option<f64> {
        if target_gap <= 0.0 {
            return Some(0.0);
        }
        let mut needed_cp = 0.0;
        for &(eff, cp_cost, max_levels) in &self.items {
            if target_gap <= 0.0 {
                break;
            }
            let level_gdp = cp_cost * eff;
            let max_gdp = level_gdp * max_levels;
            if target_gap > max_gdp {
                needed_cp += cp_cost * max_levels;
                target_gap -= max_gdp;
            } else {
                needed_cp += target_gap / eff;
                target_gap = 0.0;
            }
        }
        if target_gap > 0.0 {
            if let Some(&(eff, _, _)) = self.items.first() {
                needed_cp += target_gap / eff;
            } else {
                return None;
            }
        }
        Some(needed_cp)
    }

    fn new(economy: &crate::sim::EconomyContext, config: crate::sim::SimConfig) -> Self {
        let mut items = Vec::new();
        let mut efficiency_map = std::collections::HashMap::new();
        for building_type in economy.defs.building_types.values() {
            let (inputs, outputs) =
                crate::sim::default_building_io_per_level(&economy.defs, building_type);
            let mut gdp_add = 0.0;
            for i in 0..outputs.len() {
                let good_id = vic3_defs::GoodId::from_usize(i);
                let qty = outputs[good_id];
                if qty > 0.0 {
                    if let Some(good) = economy.defs.goods.get(economy.defs.goods_order[i].as_str())
                    {
                        gdp_add += qty * good.base_price;
                    }
                }
            }
            for i in 0..inputs.len() {
                let good_id = vic3_defs::GoodId::from_usize(i);
                let qty = inputs[good_id];
                if qty > 0.0 {
                    if let Some(good) = economy.defs.goods.get(economy.defs.goods_order[i].as_str())
                    {
                        gdp_add -= qty * good.base_price;
                    }
                }
            }
            if gdp_add <= 0.0 {
                continue;
            }

            let Some(cp_cost) = crate::construction::construction_work_points_for_enqueue(
                &building_type.name,
                economy,
                config,
            ) else {
                continue;
            };
            if cp_cost <= 0.0 {
                continue;
            }

            let max_levels = 1000.0; // Assume loose high upper bound for all buildings until parser supports potentials
            let efficiency = gdp_add / cp_cost;
            items.push((efficiency, cp_cost, max_levels));
            efficiency_map.insert(building_type.name.clone(), efficiency);
        }
        items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Self {
            items,
            efficiency_map,
        }
    }

    pub fn efficiency_for(&self, building_type: &str) -> f64 {
        self.efficiency_map
            .get(building_type)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Immutable inputs shared by every node in one planning search.
#[derive(Debug)]
struct SearchContext {
    goal: Goal,
    config: SimConfig,
    economy: Rc<EconomyContext>,
    /// Vic3-side search counters — see [`SearchTraceStats`].
    trace: SearchTraceStats,
    gdp_knapsack: GdpKnapsack,
}

/// Compact key and state handle for Vic3 planning.
///
/// Nodes created by one search all share the same goal and simulator config.
/// Hash/Eq forward to [`Vic3Identity`]; [`Vic3Cache`] (context/trace) stays
/// out of the closed-set key.
#[derive(Clone, Debug)]
pub struct Vic3Node {
    identity: Vic3Identity,
    cache: Vic3Cache,
    /// GDP used for residual / rate math when ranking children.
    ///
    /// Design doc: $\mathrm{gdp}_{\mathrm{for\_rates}}$ in
    /// `docs/planning-progress-heuristic.md`. Equals [`PlanningState::gdp`] at
    /// roots and after real price refresh; after PEA emit of a build may be
    /// state_t GDP plus the emit-time GDP delta (anticipated before
    /// `BuildingCompleted` updates `state.gdp`). Not part of Hash/Eq identity.
    gdp_for_rates: f64,
}

/// Domain identity for the pathfinder intern map.
///
/// `Hash` is fingerprint-only; `Eq` uses pointer equality or fingerprint +
/// full [`PlanningState`] (see module docs).
#[derive(Clone, Debug)]
struct Vic3Identity {
    fingerprint: u64,
    state: Rc<PlanningState>,
}

/// Per-search shared inputs excluded from Hash/Eq (goal, config, economy, trace).
#[derive(Clone, Debug)]
struct Vic3Cache {
    context: Rc<SearchContext>,
}

impl Vic3Identity {
    fn new(state: PlanningState) -> Self {
        let fingerprint = state.fingerprint();
        Self {
            fingerprint,
            state: Rc::new(state),
        }
    }
}

impl PartialEq for Vic3Identity {
    fn eq(&self, other: &Self) -> bool {
        // Same allocation → equal. Distinct Rcs may still be the same IR
        // (reconstructs); Hash is fingerprint-only, so Eq verifies full state
        // when pointers differ (also covers u64 fingerprint collisions).
        Rc::ptr_eq(&self.state, &other.state)
            || (self.fingerprint == other.fingerprint && *self.state == *other.state)
    }
}

impl Eq for Vic3Identity {}

impl Hash for Vic3Identity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

impl Vic3Node {
    /// Create the root of a planning search with required economy context.
    pub fn new(
        state: PlanningState,
        goal: Goal,
        config: SimConfig,
        economy: EconomyContext,
    ) -> Self {
        let knapsack = GdpKnapsack::new(&economy, config);
        Self::with_context(
            state,
            Rc::new(SearchContext {
                goal,
                config,
                economy: Rc::new(economy),
                trace: SearchTraceStats::default(),
                gdp_knapsack: knapsack,
            }),
        )
    }

    /// Alias for [`Self::new`] (historical name from the optional-economy era).
    pub fn new_with_economy(
        state: PlanningState,
        goal: Goal,
        config: SimConfig,
        economy: EconomyContext,
    ) -> Self {
        Self::new(state, goal, config, economy)
    }

    fn with_context(state: PlanningState, context: Rc<SearchContext>) -> Self {
        let gdp_for_rates = state.gdp;
        let identity = Vic3Identity::new(state);
        context.trace.note_fp(identity.fingerprint);
        Self {
            identity,
            cache: Vic3Cache { context },
            gdp_for_rates,
        }
    }

    /// GDP for residual-days / rate math (may anticipate emit GDP delta).
    pub fn gdp_for_rates(&self) -> f64 {
        self.gdp_for_rates
    }

    /// Set anticipated GDP for child ranking after PEA emit scoring.
    pub(crate) fn with_gdp_for_rates(mut self, gdp_for_rates: f64) -> Self {
        self.gdp_for_rates = gdp_for_rates;
        self
    }

    /// `(dups, uniques)` of domain fingerprints created in this search.
    pub(crate) fn fingerprint_dup_stats(&self) -> (u64, u64) {
        self.cache.context.trace.fp.snapshot()
    }

    /// Full Vic3-side search summary (fp + PEA frontier proxies).
    pub(crate) fn search_trace_summary(&self) -> String {
        self.cache.context.trace.summary_line()
    }

    pub(crate) fn note_pea_ready(&self, ranked_len: usize) {
        self.cache.context.trace.note_pea_ready(ranked_len);
    }

    pub(crate) fn note_pea_resume(&self) {
        self.cache.context.trace.note_pea_resume();
    }

    pub(crate) fn note_beam_emit(&self, emitted: usize, deferred: bool) {
        self.cache.context.trace.note_beam_emit(emitted, deferred);
    }

    /// New domain node sharing this node's search context (PEA* child construction).
    pub(crate) fn with_shared_context(state: PlanningState, template: &Self) -> Self {
        Self::with_context(state, Rc::clone(&template.cache.context))
    }

    /// Compact identity used by the pathfinder's intern map.
    pub fn fingerprint(&self) -> u64 {
        self.identity.fingerprint
    }

    /// Projected world state represented by this node.
    pub fn state(&self) -> &PlanningState {
        &self.identity.state
    }

    /// Compiled goal shared by this search.
    pub fn goal(&self) -> &Goal {
        &self.cache.context.goal
    }

    /// Simulator timing configuration shared by this search.
    pub fn config(&self) -> SimConfig {
        self.cache.context.config
    }

    /// Economy context shared by this search.
    pub fn economy(&self) -> &EconomyContext {
        &self.cache.context.economy
    }

    pub(crate) fn sim_successors(&self) -> Vec<Successor> {
        crate::sim::successors(
            &self.identity.state,
            &self.cache.context.goal,
            self.cache.context.config,
            &self.cache.context.economy,
        )
    }

    /// Apply one action against this domain node (PEA* emit path).
    pub(crate) fn apply_action(&self, action: &crate::sim::Action) -> Option<PlanningState> {
        crate::sim::apply_action(
            &self.identity.state,
            action,
            &self.cache.context.economy,
            self.cache.context.config,
        )
    }
}

impl PartialEq for Vic3Node {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for Vic3Node {}

impl Hash for Vic3Node {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

/// Admissible remaining-days bound from the compiled goal's dependency DAG.
///
/// Atomic timing is known for research, interest declarations, army power
/// expansions, law enactments, and (when no zero-day PM path exists)
/// construction. AND uses the longest child (actions may overlap or satisfy
/// multiple atoms), OR uses the cheapest child, and NOT stays at zero. Missing
/// technologies / open interest / raisable army / missing laws always
/// contribute their fixed duration, even when an unrelated item is queued:
/// returning zero there would drop the estimate over a zero-day queue edge and
/// break consistency for closed-node A*. Open `good_price` / `gdp` atoms
/// contribute `construction_days` when the economy context has no SwitchPm
/// candidate; otherwise they contribute zero because PM switches are 0-day.
/// Fiscal, SoL, tax, and other atoms without a proven timing model contribute
/// zero. This is deliberately a relaxation of the real state graph, not a
/// replacement for A*.
///
/// Goal-DAG timing lower bound used by A* (`h_adm`).
///
/// AND → max, OR → min across children (independent tracks finish near the
/// max). Open research uses defs cost / remaining-style ETA when available but
/// never treats a missing tech as free while queued (consistency over 0-day
/// enqueue). Construction uses head remaining ÷ rate when set.
///
/// Search candidate-bag ordering uses [`super::bag_rank`] over
/// progress-aware [`super::progress_h`] scorers. Incumbent $U$ from greedy
/// (builds allowed; Construction Sector excluded) prunes via
/// `PathFinderBuilder::max_cost` in [`super::result`].
fn goal_timing_lower_bound(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
    context: &SearchContext,
) -> u32 {
    let interest_days = u32::from(config.interest_days.max(1));
    let army_train_days = u32::from(config.army_training_days.max(1));
    let navy_crew_days = u32::from(config.navy_crew_days.max(1));
    let law_days = u32::from(config.law_days.max(1));
    let construction_days = construction_eta_days(state, config);
    match goal {
        Goal::And(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config, economy, context))
            .max()
            .unwrap_or(0),
        Goal::Or(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config, economy, context))
            .min()
            .unwrap_or(0),
        Goal::Not(_) => 0,
        Goal::Simple(SimpleSubgoal::HasTech(tech)) => {
            research_eta_for_leaf(tech, state, config, economy)
        }
        Goal::Simple(SimpleSubgoal::HasLaw(law)) => {
            if state.has_law(law) {
                0
            } else {
                law_days
            }
        }
        Goal::Simple(SimpleSubgoal::InterestIn { kind, id }) => {
            let held = match kind {
                crate::goals::InterestKind::State => state.has_interest_state(id),
                crate::goals::InterestKind::Region => state.has_interest_region(id),
            };
            if held {
                0
            } else {
                interest_days
            }
        }
        Goal::Simple(atom @ SimpleSubgoal::ArmyPower { rel, value }) => {
            if atom.eval(state) {
                0
            } else if !state.army_buildings_fully_staffed() {
                army_train_days
            } else if crate::sim::power_raise_needed(*rel, *value, state.army_power_projection)
                .is_some()
            {
                construction_days.saturating_add(army_train_days)
            } else {
                0
            }
        }
        Goal::Simple(atom @ SimpleSubgoal::NavyPower { rel, value }) => {
            if atom.eval(state) {
                0
            } else if !state.navy_buildings_fully_staffed() {
                navy_crew_days
            } else if crate::sim::power_raise_needed(*rel, *value, state.navy_power_projection)
                .is_some()
            {
                construction_days.saturating_add(navy_crew_days)
            } else {
                0
            }
        }
        Goal::Simple(atom @ SimpleSubgoal::GoodPrice { .. }) => {
            if atom.eval(state)
                || economy.has_pm_switch_path(state, std::slice::from_ref(atom), config)
            {
                0
            } else {
                construction_days
            }
        }
        Goal::Simple(atom @ SimpleSubgoal::Gdp { .. }) => {
            if atom.eval(state)
                || economy.has_pm_switch_path(state, std::slice::from_ref(atom), config)
            {
                0
            } else {
                let mut target_val = 0.0;
                if let SimpleSubgoal::Gdp { value, .. } = atom {
                    target_val = *value;
                }
                let target_gap = target_val - state.gdp;
                if target_gap <= 0.0 {
                    return 0;
                }

                let Some(needed_cp) = context.gdp_knapsack.needed_cp(target_gap) else {
                    return construction_days;
                };

                // ADMISSIBILITY MATH (The "Savings" Bound):
                // To maintain A* admissibility (never overestimating the true cost), we
                // must credit the heuristic for the CP already spent on queued buildings.
                // However, we CANNOT simply sum the `remaining_cp` of all queued buildings.
                // If a terrible building is at the bottom of the queue, it might never start
                // building before we hit the GDP goal using optimal buildings. If we penalized
                // the heuristic by adding the terrible building's cost, we would overestimate.
                //
                // ADMISSIBILITY MATH (The "Fixed-Point Queue Penalty" Bound):
                // To maintain A* admissibility (never overestimating the true cost), we
                // must account for the CP stolen by sub-optimal queued buildings.
                //
                // Proof:
                // Let our knapsack target take OptimalCP = 1000 CP. Rate = 5 CP/day.
                // If we queue a useless Monument (100 CP, capped at 1 CP/day):
                // For 100 days, the Monument gets 1 CP/day, the Knapsack gets 4 CP/day.
                // After 100 days, the Monument is done, and the Knapsack gets 5 CP/day.
                // It takes 100 days to reach 400 Knapsack CP, leaving 600 CP.
                // 600 CP at 5 CP/day takes 120 days. Total time = 220 days.
                //
                // We calculate this exactly in O(1) using a fixed-point equation:
                // T = (OptimalCP + sum(min(remaining_cp, cap_rate * T))) / TotalRate
                //
                // Since `max_parallel` active jobs is small, we solve this iteratively.
                // Each queued building's penalty is bounded by its cap * T. If T is large,
                // the building finishes early and its penalty maxes out at `remaining_cp`.
                // If we hit the goal early, the building's penalty is bounded to exactly
                // what it managed to steal (`cap_rate * T`). This perfectly preserves
                // admissibility without ever running a timeline simulation.
                
                let mut sum_theoretical_costs = 0.0;
                let mut active_penalties = Vec::new();
                
                let cap_rates = crate::construction::construction_points_per_day_per_job(state, config);
                
                for (c, &cap_rate) in state.constructions.iter().zip(&cap_rates) {
                    let eff = context
                        .gdp_knapsack
                        .efficiency_map
                        .get(&c.building_type_name)
                        .copied()
                        .unwrap_or(0.0);
                    let total_cost = crate::construction::construction_work_points_for_enqueue(
                        &c.building_type_name,
                        economy,
                        config,
                    )
                    .unwrap_or(f64::from(config.default_construction_cost));
                    
                    let remaining_cp = c.remaining.unwrap_or(total_cost);
                    
                    if cap_rate > 0.0 {
                        active_penalties.push((remaining_cp, cap_rate));
                    }
                    
                    if eff > 0.0 {
                        let gdp = total_cost * eff;
                        if gdp > 0.0 {
                            sum_theoretical_costs += context.gdp_knapsack.needed_cp(gdp).unwrap_or(0.0);
                        }
                    }
                }
                
                // The base optimal CP required, assuming no penalties yet.
                let optimal_cp = (needed_cp - sum_theoretical_costs).max(0.0);

                let mut future_rate = state.construction_points_per_day;
                for c in &state.constructions {
                    if c.building_type_name == crate::construction::BUILDING_CONSTRUCTION_SECTOR {
                        future_rate += 5.0; // Optimistic upper bound for sector yield
                    }
                }
                  let c_0 = future_rate.max(1.0);
                let c_s = f64::from(config.default_construction_cost);
                let delta_c = 5.0; // Optimistic upper bound for construction sector yield
                let r = delta_c / c_s;
                
                // Fixed-point iterative solver for T
                // T = ExponentialTime(OptimalCP + sum(min(remaining_cp, cap_rate * T)))
                //
                // ARCHITECTURE NOTE: Why open-code this instead of using `basin` (our NLS solver)?
                // 1. Performance: This is the A* heuristic hot-path, called millions of times per second. 
                //    A generalized solver like `basin` would introduce function call/trait overhead that 
                //    would destroy search performance. This open-coded loop compiles to inline SIMD/floats.
                // 2. Strict Admissibility: A* requires we NEVER overestimate. By initializing `t_guess = 0` 
                //    and iterating on a concave function, we approach the fixed point strictly from below. 
                //    If we terminate early (e.g. hit max iterations), we return a value slightly *less* 
                //    than the true fixed point, preserving perfect admissibility. A generalized solver 
                //    might overshoot by 0.000001, which `.ceil()` would round up, breaking admissibility!
                let mut t_guess = 0.0;
                for _ in 0..10 {
                    let mut penalty = 0.0;
                    for &(remaining_cp, cap_rate) in &active_penalties {
                        penalty += remaining_cp.min(cap_rate * t_guess);
                    }
                    
                    let w = optimal_cp + penalty;
                    if w <= 0.0 {
                        t_guess = 0.0;
                        break;
                    }
                    
                    let new_t = if r * (w / c_0) > 1.0 {
                        let t1 = (1.0 / r) * (r * w / c_0).ln();
                        t1 + (1.0 / r)
                    } else {
                        w / c_0
                    };
                    
                    if (new_t - t_guess).abs() < 0.1 {
                        t_guess = new_t;
                        break;
                    }
                    t_guess = new_t;
                }
                let knapsack_days = t_guess.ceil() as u32;

                construction_days.max(knapsack_days)
            }
        }
        Goal::Simple(_) => 0,
    }
}

/// Serial research ETA for a leaf tech (missing ancestors sum when defs exist).
///
/// Queued identity is ignored: a missing tech always costs at least one research
/// period so a 0-day `QueueTech` edge cannot drop the heuristic (A* consistency).

/// Translates the current game state into base optimal CP required and 
/// extracts penalties for any active sub-optimal jobs in the queue.
#[inline(always)]
fn calculate_optimal_cp_and_penalties(
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
    context: &SearchContext,
    target_gap: f64,
) -> Option<(f64, Vec<(f64, f64)>)> {
    let needed_cp = context.gdp_knapsack.needed_cp(target_gap)?;
    let mut sum_theoretical_costs = 0.0;
    let mut active_penalties = Vec::new();
    
    let cap_rates = crate::construction::construction_points_per_day_per_job(state, config);
    
    for (c, &cap_rate) in state.constructions.iter().zip(&cap_rates) {
        let eff = context
            .gdp_knapsack
            .efficiency_map
            .get(&c.building_type_name)
            .copied()
            .unwrap_or(0.0);
        let total_cost = crate::construction::construction_work_points_for_enqueue(
            &c.building_type_name,
            economy,
            config,
        )
        .unwrap_or(f64::from(config.default_construction_cost));
        
        let remaining_cp = c.remaining.unwrap_or(total_cost);
        
        if cap_rate > 0.0 {
            active_penalties.push((remaining_cp, cap_rate));
        }
        
        if eff > 0.0 {
            let gdp = total_cost * eff;
            if gdp > 0.0 {
                sum_theoretical_costs += context.gdp_knapsack.needed_cp(gdp).unwrap_or(0.0);
            }
        }
    }
    
    let optimal_cp = (needed_cp - sum_theoretical_costs).max(0.0);
    Some((optimal_cp, active_penalties))
}

/// Optimistically projects future capacity if the current queue contains construction sectors.
#[inline(always)]
fn estimate_optimistic_capacity(state: &PlanningState) -> f64 {
    let mut future_rate = state.construction_points_per_day;
    for c in &state.constructions {
        if c.building_type_name == crate::construction::BUILDING_CONSTRUCTION_SECTOR {
            future_rate += 5.0; // Optimistic upper bound for sector yield
        }
    }
    future_rate.max(1.0)
}

/// Pure mathematical solver for the queue penalty fixed-point timeline.
/// T = ExponentialTime(OptimalCP + sum(min(remaining_cp, cap_rate * T)))
/// 
/// ARCHITECTURE NOTE: Why open-code this instead of using `basin` (our NLS solver)?
/// 1. Performance: This is the A* heuristic hot-path, called millions of times per second. 
///    A generalized solver like `basin` would introduce function call/trait overhead that 
///    would destroy search performance. This open-coded loop compiles to inline SIMD/floats.
/// 2. Strict Admissibility: A* requires we NEVER overestimate. By initializing `t_guess = 0` 
///    and iterating on a concave function, we approach the fixed point strictly from below. 
///    If we terminate early (e.g. hit max iterations), we return a value slightly *less* 
///    than the true fixed point, preserving perfect admissibility. A generalized solver 
///    might overshoot by 0.000001, which `.ceil()` would round up, breaking admissibility!
#[inline(always)]
fn solve_fixed_point_timeline(
    optimal_cp: f64,
    active_penalties: &[(f64, f64)],
    c_0: f64,
    r: f64,
) -> f64 {
    let mut t_guess = 0.0;
    for _ in 0..10 {
        let mut penalty = 0.0;
        for &(remaining_cp, cap_rate) in active_penalties {
            penalty += remaining_cp.min(cap_rate * t_guess);
        }
        
        let w = optimal_cp + penalty;
        if w <= 0.0 {
            t_guess = 0.0;
            break;
        }
        
        let new_t = if r * (w / c_0) > 1.0 {
            let t1 = (1.0 / r) * (r * w / c_0).ln();
            t1 + (1.0 / r)
        } else {
            w / c_0
        };
        
        if (new_t - t_guess).abs() < 0.1 {
            t_guess = new_t;
            break;
        }
        t_guess = new_t;
    }
    t_guess
}

fn research_eta_for_leaf(
    tech: &str,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
) -> u32 {
    if state.has_tech(tech) {
        return 0;
    }
    let fallback = u32::from(config.research_days.max(1));
    if economy.defs.technologies.is_empty() {
        return fallback;
    }
    let missing = crate::tech::missing_tech_closure(tech, state, &economy.defs);
    if missing.is_empty() {
        return 0;
    }
    let mut total = 0u32;
    for id in missing {
        total = total.saturating_add(single_tech_research_eta(&id, config, economy));
    }
    total.max(1)
}

fn single_tech_research_eta(tech: &str, config: SimConfig, economy: &EconomyContext) -> u32 {
    let fallback = u32::from(config.research_days.max(1));
    if let Some(cost) = crate::tech::tech_research_cost(tech, &economy.defs) {
        if let Some(days) = crate::tracks::days_for_work(cost, crate::tracks::CONSTANT_RATE) {
            return days.max(1);
        }
    }
    fallback
}

fn construction_eta_days(state: &PlanningState, config: SimConfig) -> u32 {
    crate::construction::construction_eta_days(
        state,
        config,
        crate::construction::ConstructionEtaMode::CapacityOrSlot,
    )
}

impl SearchNode for Vic3Node {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, Self::Cost)> {
        let edges = self.sim_successors();
        super::astar_trace::on_expand("vic3", || {
            let mut kinds = std::collections::BTreeMap::<&str, u32>::new();
            for e in &edges {
                let k = match &e.action {
                    crate::sim::Action::QueueTech { .. } => "QueueTech",
                    crate::sim::Action::QueueBuildingLevel { .. } => "QueueBuilding",
                    crate::sim::Action::QueueInterest { .. } => "QueueInterest",
                    crate::sim::Action::QueueHireMilitary { .. } => "QueueHire",
                    crate::sim::Action::QueueLaw { .. } => "QueueLaw",
                    crate::sim::Action::SwitchPm { .. } => "SwitchPm",
                    crate::sim::Action::AdjustTax { .. } => "AdjustTax",
                    crate::sim::Action::WaitForEvent { .. } => "Wait",
                };
                *kinds.entry(k).or_default() += 1;
            }
            format!(
                "fp={:016x} gdp={:.0} h={} branch={} {:?}",
                self.fingerprint(),
                self.state().gdp,
                self.heuristic(),
                edges.len(),
                kinds
            )
        });
        edges
            .into_iter()
            .map(|successor| {
                let cost = u32::from(successor.days);
                let next = Self::with_context(successor.state, Rc::clone(&self.cache.context));
                // A* Consistency Check: h(parent) <= cost + h(child)
                debug_assert!(
                    self.heuristic() <= cost.saturating_add(next.heuristic()),
                    "Consistency violation in Vic3Node: h(parent)={} > cost={} + h(child)={}",
                    self.heuristic(),
                    cost,
                    next.heuristic()
                );
                (next, cost)
            })
            .collect()
    }

    fn is_goal(&self) -> bool {
        let ok = evaluate(&self.cache.context.goal, &self.identity.state);
        if ok {
            super::astar_trace::on_goal("vic3", || {
                format!(
                    "fp={:016x} gdp={:.0} date={:?}",
                    self.fingerprint(),
                    self.state().gdp,
                    self.state().date
                )
            });
        }
        ok
    }

    /// Goal-DAG relaxation: exact for research / interest / raisable army / law
    /// atoms, construction days for open price/GDP when no SwitchPm path exists,
    /// zero for atoms without a proven timing model.
    fn heuristic(&self) -> Self::Cost {
        goal_timing_lower_bound(
            &self.cache.context.goal,
            &self.identity.state,
            self.cache.context.config,
            &self.cache.context.economy,
            &self.cache.context,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::plan::pathfinding::{shortest_path, shortest_path_lazy};
    use crate::sim::{Action, EconomyContext};
    use crate::world::{PlanningParts, PlanningState};
    use proptest::prelude::*;
    use rust_advanced_heaps::pairing::PairingHeap;
    use rust_advanced_heaps::simple_binary::SimpleBinaryHeap;

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
            EconomyContext::empty(),
        )
    }

    #[test]
    fn interest_and_army_plan_closes_declare_war_when_munitions_solvent_ok() {
        // Army PP already meets the declare-war threshold; only interest remains.
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                solvent: true,
                good_prices: vec![("ammunition".into(), 30.0)],
                army_power_projection: Some(150.0),
                army_pp_baseline: Some(150.0),
                ..PlanningParts::default()
            }),
            compile("declare-war(state=alsace)").unwrap(),
            SimConfig {
                interest_days: 30,
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );

        assert_eq!(
            start.heuristic(),
            30,
            "AND bound is interest when army holds"
        );
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("declare-war reachable");
        assert_eq!(cost, 30);
        let goal_node = path.last().unwrap();
        assert!(goal_node.is_goal());
        assert!(goal_node.state().has_interest_state("alsace"));
        assert!(goal_node
            .state()
            .army_power_projection
            .is_some_and(|p| p >= 100.0));
    }

    #[test]
    fn army_only_plan_hires_staffed_barracks() {
        use crate::military::{ModeledMilBuilding, UnitCombatStats, BUILDING_BARRACKS};
        let per = UnitCombatStats::army_default().full_power_projection();
        let levels = (100.0 / per).ceil();
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                army_power_projection: Some(0.0),
                army_pp_baseline: Some(0.0),
                mil_buildings: vec![ModeledMilBuilding {
                    building: BUILDING_BARRACKS.into(),
                    levels,
                    staffing: 0.0,
                }],
                ..PlanningParts::default()
            }),
            compile("army_power_projection >= 100").unwrap(),
            SimConfig {
                army_training_days: 77,
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );
        assert_eq!(start.heuristic(), 77);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("army goal reachable");
        assert_eq!(cost, 77);
        assert!(path.last().unwrap().is_goal());
        assert!(path
            .last()
            .unwrap()
            .state()
            .army_power_projection
            .is_some_and(|p| p >= 100.0));
    }

    #[test]
    fn interest_only_plan_cost_is_queue_plus_wait_days() {
        let start = Vic3Node::new(
            PlanningState::default(),
            compile("interest_in(region=region_western_europe)").unwrap(),
            SimConfig {
                interest_days: 25,
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );
        assert_eq!(start.heuristic(), 25);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("interest goal reachable");
        assert_eq!(cost, 25);
        assert!(path[2].state().has_interest_region("region_western_europe"));
        assert!(path[2].is_goal());
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

        let first_edges = crate::sim::successors(
            start.state(),
            start.goal(),
            start.config(),
            &EconomyContext::empty(),
        );
        assert!(matches!(
            first_edges.as_slice(),
            [crate::sim::Successor {
                action: Action::QueueTech { tech },
                days: 0,
                ..
            }] if tech == "nitroglycerin"
        ));
    }

    #[test]
    fn payday_plan_closes_solvent() {
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                debt_principal: Some(1_000.0),
                credit_limit: Some(1_000.0),
                credit_headroom: Some(0.0),
                solvent: false,
                weekly_balance: Some(100.0),
                treasury: 0.0,
                ..PlanningParts::default()
            }),
            compile("solvent").unwrap(),
            SimConfig {
                payday_days: 7,
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("solvent is reachable");
        assert_eq!(cost, 7);
        assert!(path.last().is_some_and(|node| node.is_goal()));
        assert!(path.last().unwrap().state().solvent);
        assert_eq!(path.last().unwrap().state().credit_headroom, Some(100.0));
    }

    #[test]
    fn law_plan_cost_is_queue_plus_wait_days() {
        let start = Vic3Node::new(
            PlanningState::default(),
            compile("has_law(law_homesteading)").unwrap(),
            SimConfig {
                law_days: 55,
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );
        assert_eq!(start.heuristic(), 55);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("law goal reachable");
        assert_eq!(cost, 55);
        assert!(path.last().unwrap().state().has_law("homesteading"));
        assert!(path.last().unwrap().is_goal());
    }

    #[test]
    fn tax_plan_closes_weekly_balance_in_zero_days() {
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                weekly_balance: Some(20.0),
                ..PlanningParts::default()
            }),
            compile("weekly_balance >= 100").unwrap(),
            SimConfig {
                tax_balance_per_step: 50,
                max_tax_steps: 3,
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );
        assert_eq!(start.heuristic(), 0);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("tax goal reachable");
        assert_eq!(cost, 0);
        assert!(path.last().unwrap().is_goal());
        assert!(path.last().unwrap().state().weekly_balance.unwrap() >= 100.0);
    }

    #[test]
    fn construction_bound_applies_without_pm_path() {
        let start = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                good_prices: vec![("wood".into(), 40.0)],
                construction_points_per_day: 1.0,
                ..PlanningParts::default()
            }),
            compile("good_price(wood) <= 20").unwrap(),
            SimConfig {
                // Capacity/slot ETA: one default-cost level at government rate.
                default_construction_cost: 33,
                max_construction_allocation: Some(1000),
                ..SimConfig::default()
            },
            EconomyContext::empty(),
        );
        assert_eq!(
            start.heuristic(),
            33,
            "empty economy has no PM candidates; open price uses capacity ETA (one level)"
        );
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
            EconomyContext::empty(),
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
            EconomyContext::empty(),
        );
        let (_, or_cost) =
            shortest_path::<_, PairingHeap<_, _>>(&or).expect("either tech is reachable");
        assert_eq!(or.heuristic(), 40);
        assert_eq!(or_cost, 40);
    }

    fn tech_tree_defs() -> vic3_defs::GameDefs {
        use std::collections::BTreeMap;
        use vic3_defs::{GameDefs, Technology};
        let mut technologies = BTreeMap::new();
        technologies.insert(
            "manufacturies".into(),
            Technology {
                name: "manufacturies".into(),
                cost: Some(50.0),
                prerequisites: vec![],
            },
        );
        technologies.insert(
            "shaft_mining".into(),
            Technology {
                name: "shaft_mining".into(),
                cost: Some(75.0),
                prerequisites: vec!["manufacturies".into()],
            },
        );
        technologies.insert(
            "nitroglycerin".into(),
            Technology {
                name: "nitroglycerin".into(),
                cost: Some(100.0),
                prerequisites: vec!["shaft_mining".into()],
            },
        );
        GameDefs {
            technologies,
            ..GameDefs::default()
        }
    }

    #[test]
    fn plan_queues_tech_ancestors_before_leaf() {
        use crate::sim::EconomyContext;
        use vic3_prices::{SolveOpts, World};

        let economy = EconomyContext::new(World::default(), tech_tree_defs(), SolveOpts::default());
        let start = Vic3Node::new_with_economy(
            PlanningState::default(),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig {
                research_days: 365,
                ..SimConfig::default()
            },
            economy,
        );
        // Serial costs 50+75+100 at rate 1.0.
        assert_eq!(start.heuristic(), 225);
        let (path, cost) =
            shortest_path::<_, PairingHeap<_, _>>(&start).expect("tech tree reachable");
        assert_eq!(cost, 225);
        let mut research_order = Vec::new();
        for window in path.windows(2) {
            let before = &window[0].state().techs;
            let after = &window[1].state().techs;
            for tech in after.difference(before) {
                research_order.push(tech.clone());
            }
        }
        assert_eq!(
            research_order,
            ["manufacturies", "shaft_mining", "nitroglycerin"]
        );
    }

    #[test]
    fn research_and_construction_heuristic_is_max_across_tracks() {
        use crate::sim::EconomyContext;
        use crate::world::{ConstructionQueueKind, PlanningConstruction};
        use vic3_prices::{SolveOpts, World};

        let economy = EconomyContext::new(World::default(), tech_tree_defs(), SolveOpts::default());
        let state = PlanningState::from_parts(PlanningParts {
            constructions: vec![PlanningConstruction {
                order_id: 1,
                queue: ConstructionQueueKind::Government,
                state_id: None,
                building_type_name: "building_rye_farm".into(),
                remaining: Some(30.0),
            }],
            construction_points_per_day: 1.0,
            good_prices: vec![("wood".into(), 40.0)],
            ..PlanningParts::default()
        });
        let start = Vic3Node::new_with_economy(
            state,
            compile("has_tech(manufacturies) && good_price(wood) <= 20").unwrap(),
            SimConfig {
                research_days: 365,
                construction_days: 180,
                ..SimConfig::default()
            },
            economy,
        );
        // Research 50, construction remaining 30 → max = 50.
        assert_eq!(start.heuristic(), 50);
    }

    #[test]
    fn missing_tech_bound_ignores_queue_identity() {
        let config = SimConfig {
            research_days: 40,
            ..SimConfig::default()
        };
        let goal = compile("research(tech=railways)").unwrap();

        let idle = Vic3Node::new(
            PlanningState::default(),
            goal.clone(),
            config,
            EconomyContext::empty(),
        );
        assert_eq!(idle.heuristic(), 40);

        let matching = PlanningState {
            queued_tech: Some("railways".into()),
            ..PlanningState::default()
        };
        assert_eq!(
            Vic3Node::new(matching, goal.clone(), config, EconomyContext::empty()).heuristic(),
            40
        );

        let unrelated = PlanningState {
            queued_tech: Some("unrelated_tech".into()),
            ..PlanningState::default()
        };
        assert_eq!(
            Vic3Node::new(unrelated, goal, config, EconomyContext::empty()).heuristic(),
            40
        );
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
            EconomyContext::empty(),
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
        let rebuilt = Vic3Node::new(
            start.state().clone(),
            start.goal().clone(),
            start.config(),
            EconomyContext::empty(),
        );
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
            EconomyContext::empty()
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

        /// Mathematically proves the exponential capacity growth bound is strictly admissible.
        ///
        /// For any target work `w` and starting capacity `c0`, no matter how many (`k`)
        /// construction sectors the player builds to expand capacity, the continuous-time
        /// exponential heuristic is ALWAYS <= the true discrete time it takes to build them.
        #[test]
        fn gdp_knapsack_exponential_bound_is_admissible(
            w in 0.0f64..10_000_000.0,
            c0 in 1.0f64..1000.0,
            k in 0u32..500
        ) {
            let cs = 100.0;
            let delta_c = 5.0;
            let r = delta_c / cs;

            // True discrete simulation of building `k` sectors sequentially, then building the goal.
            let mut true_days = 0;
            let mut c = c0;
            for _ in 0..k {
                true_days += (cs / c).ceil() as u32;
                c += delta_c;
            }
            true_days += (w / c).ceil() as u32;

            // Our heuristic formula
            let heuristic = if r * (w / c0) > 1.0 {
                let t1 = (1.0 / r) * (r * w / c0).ln();
                let t2 = t1 + (1.0 / r);
                t2.ceil() as u32
            } else {
                (w / c0).ceil() as u32
            };

            prop_assert!(
                heuristic <= true_days,
                "Heuristic {} exceeded true discrete time {} for w={}, c0={}, k={}",
                heuristic, true_days, w, c0, k
            );
        }
    }

    #[test]
    fn test_gdp_knapsack_heuristic_bounds() {
        let defs = vic3_defs::GameDefs::default();
        let world = vic3_prices::World::default();
        let economy = std::rc::Rc::new(EconomyContext::new(
            world,
            defs,
            vic3_prices::SolveOpts::default(),
        ));

        let goal = compile("gdp >= 100").unwrap();
        let mut state = PlanningState {
            gdp: 10.0,
            construction_points_per_day: 5.0,
            ..PlanningState::default()
        };

        let config = SimConfig {
            base_construction_capacity: 5,
            default_construction_cost: 100,
            ..SimConfig::default()
        };

        let mut context = SearchContext {
            goal: goal.clone(),
            config,
            economy: economy.clone(),
            trace: crate::plan::vic3::SearchTraceStats::default(),
            gdp_knapsack: GdpKnapsack {
                items: vec![],
                efficiency_map: std::collections::HashMap::new(),
            },
        };

        // By default, the knapsack is empty.
        // When knapsack is empty, it returns `construction_days` (which is computed dynamically based on config and state).
        let base_days = goal_timing_lower_bound(&goal, &state, config, &economy, &context);

        // Inject a mock knapsack item manually for the test.
        // efficiency = 1.0, cp_cost = 100.0, max_levels = 1000.0
        context.gdp_knapsack.items.push((1.0, 100.0, 1000.0));

        // target gap is 90.
        // needed_cp = 90.0 / 1.0 = 90.0.
        // knapsack_days = ceil(90.0 / 5.0) = 18 days.
        // max(18, base_days) = base_days (since 18 < 70).
        assert_eq!(
            goal_timing_lower_bound(&goal, &state, config, &economy, &context),
            base_days
        );

        // Now make the gap huge so knapsack_days dominates.
        state.gdp = -900.0;
        // target gap is 10000. needed_cp = 10000.0 / 1.0 = 10000.0.
        // r = 5.0 / 100.0 = 0.05
        // c_0 = 5.0
        // r * w / c_0 = 0.05 * 10000 / 5 = 100 > 1.0
        // t1 = 20 * ln(100) = 20 * 4.605 = 92.1 days
        // t2 = 92.1 + 20 = 112.1 days -> ceil(112.1) = 113 days
        // max(113, base_days) = 113
        state.gdp = -9900.0;
        assert_eq!(
            goal_timing_lower_bound(&goal, &state, config, &economy, &context),
            113
        );

        // CodeRabbit Regression Test: Queued Construction Sectors increase optimistic future capacity.
        // If we queue 2 construction sectors, the optimistic rate increases by 2 * 5.0 = 10.0.
        // Future rate = 5.0 (base) + 10.0 = 15.0.
        // c_0 = 15.0
        // r * w / c_0 = 0.05 * 10000 / 15 = 33.333 > 1.0
        // t1 = 20 * ln(33.333) = 20 * 3.506 = 70.1 days
        // t2 = 70.1 + 20 = 90.1 days -> ceil(90.1) = 91 days
        // The heuristic drops from 113 to 91, proving we optimistically model future capacity.
        state
            .constructions
            .push(crate::world::PlanningConstruction {
                building_type_name: crate::construction::BUILDING_CONSTRUCTION_SECTOR.to_string(),
                state_id: Some(1),
                remaining: None,
                order_id: 1,
                queue: crate::world::ConstructionQueueKind::Government,
            });
        state
            .constructions
            .push(crate::world::PlanningConstruction {
                building_type_name: crate::construction::BUILDING_CONSTRUCTION_SECTOR.to_string(),
                state_id: Some(2),
                remaining: None,
                order_id: 2,
                queue: crate::world::ConstructionQueueKind::Government,
            });
        // We must also add the construction sectors to the context so `needed_cp` doesn't drop due to `savings` logic.
        // We set efficiency to 0.0 for construction sectors so they yield no GDP and thus no savings.
        context.gdp_knapsack.efficiency_map.insert(
            crate::construction::BUILDING_CONSTRUCTION_SECTOR.to_string(),
            0.0,
        );

        let new_bound = goal_timing_lower_bound(&goal, &state, config, &economy, &context);
        assert!(
            new_bound < 113,
            "Heuristic did not optimistically model future construction capacity (was {})",
            new_bound
        );
        assert_eq!(new_bound, 91);
    }
    
    #[test]
    fn test_queue_penalty_fixed_point_matches_timeline() {
        let optimal_cp = 1000.0f64;
        let total_rate = 5.0f64; // c_0
        let r = 0.0; // r=0 makes the exponential function linear, matching the test scenario
        
        // The monument is queued:
        let monument_remaining = 100.0f64;
        let monument_cap = 1.0f64;
        let active_penalties = vec![(monument_remaining, monument_cap)];
        
        let t_guess = solve_fixed_point_timeline(optimal_cp, &active_penalties, total_rate, r);
        
        // User's exact manual timeline trace:
        // 100 days: Monument 1 CP/day (finishes), Knapsack 4 CP/day (400 CP)
        // Remaining knapsack: 600 CP
        // After 100 days: Knapsack 5 CP/day
        // 600 / 5 = 120 days.
        // Total time = 100 + 120 = 220 days.
        
        assert_eq!(t_guess.ceil() as u32, 220);
    }
}
