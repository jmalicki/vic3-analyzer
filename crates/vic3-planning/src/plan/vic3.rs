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

/// Immutable inputs shared by every node in one planning search.
#[derive(Debug)]
struct SearchContext {
    goal: Goal,
    config: SimConfig,
    economy: Rc<EconomyContext>,
    /// Vic3-side search counters — see [`SearchTraceStats`].
    trace: SearchTraceStats,
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
        Self::with_context(
            state,
            Rc::new(SearchContext {
                goal,
                config,
                economy: Rc::new(economy),
                trace: SearchTraceStats::default(),
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
        let identity = Vic3Identity::new(state);
        context.trace.note_fp(identity.fingerprint);
        Self {
            identity,
            cache: Vic3Cache { context },
        }
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
/// // TODO(anytime-ub): `h` is a lower bound only. A later PR can compute a
/// // greedy feasible path for easy goal shapes, take its cost as incumbent
/// // `U`, and prune nodes/edges with `g + h >= U`. That complements wide
/// // state-scoped building candidate sets (no type-level IO dominance prune).
///
/// Goal-DAG timing lower bound used by A*.
///
/// AND → max, OR → min across children (independent tracks finish near the
/// max). Open research uses defs cost / remaining-style ETA when available but
/// never treats a missing tech as free while queued (consistency over 0-day
/// enqueue). Construction uses head remaining ÷ rate when set.
fn goal_timing_lower_bound(
    goal: &Goal,
    state: &PlanningState,
    config: SimConfig,
    economy: &EconomyContext,
) -> u32 {
    let interest_days = u32::from(config.interest_days.max(1));
    let army_train_days = u32::from(config.army_training_days.max(1));
    let navy_crew_days = u32::from(config.navy_crew_days.max(1));
    let law_days = u32::from(config.law_days.max(1));
    let construction_days = construction_eta_days(state, config);
    match goal {
        Goal::And(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config, economy))
            .max()
            .unwrap_or(0),
        Goal::Or(children) => children
            .iter()
            .map(|child| goal_timing_lower_bound(child, state, config, economy))
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
        Goal::Simple(atom @ (SimpleSubgoal::GoodPrice { .. } | SimpleSubgoal::Gdp { .. })) => {
            if atom.eval(state)
                || economy.has_pm_switch_path(state, std::slice::from_ref(atom), config)
            {
                0
            } else {
                construction_days
            }
        }
        Goal::Simple(_) => 0,
    }
}

/// Serial research ETA for a leaf tech (missing ancestors sum when defs exist).
///
/// Queued identity is ignored: a missing tech always costs at least one research
/// period so a 0-day `QueueTech` edge cannot drop the heuristic (A* consistency).
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
                (
                    Self::with_context(successor.state, Rc::clone(&self.cache.context)),
                    u32::from(successor.days),
                )
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
                    type_id: BUILDING_BARRACKS.into(),
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
                id: "manufacturies".into(),
                cost: Some(50.0),
                prerequisites: vec![],
            },
        );
        technologies.insert(
            "shaft_mining".into(),
            Technology {
                id: "shaft_mining".into(),
                cost: Some(75.0),
                prerequisites: vec!["manufacturies".into()],
            },
        );
        technologies.insert(
            "nitroglycerin".into(),
            Technology {
                id: "nitroglycerin".into(),
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
                type_id: "building_rye_farm".into(),
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
    }
}
