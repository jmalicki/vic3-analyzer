//! Partial-expansion A* adapter over [`Vic3Node`].
//!
//! Encodes PEA* as distinct [`SearchNode`] identities so the existing
//! closed-set `shortest_path` loop can defer surplus successors without a
//! heaps-crate change: after emitting a fixed-size beam, the current node is
//! re-inserted via a 0-cost self-edge whose heuristic matches the next
//! child's \(f - g = \mathrm{edge} + h\).
//!
//! # Country-wide top-K bag
//!
//! Each Ready expand builds one **national** candidate bag via
//! [`super::bag_rank::cheap_rank_bag`], scores each row with open-set
//! `edge + h(child)` after speculative apply, takes top‑[`DEFAULT_PEA_BEAM`]
//! with `select_nth`, and defers the rest in an `Expanding` cursor whose
//! heuristic is the best deferred open delta. Cheap bag keys remain for
//! diagnostics only.
//!
//! Expanding identity is `(domain, emitted)` — the candidate list is
//! `Rc<[…]>` so [`SearchNode`] clones are refcount bumps, and is not hashed.

use super::bag_rank::{self, RankedBagEntry};
use super::pathfinding::SearchNode;
use super::Vic3Node;
use derivative::Derivative;
use std::cmp::Ordering;
use std::rc::Rc;

/// Fixed country-wide beam width for PEA* partial expansion.
///
/// Chosen from a no-refit GDP proxy on Prussia 1836 (~50 build-level
/// candidates; top 8–16 captured the high-value head). Not a proven optimum.
pub const DEFAULT_PEA_BEAM: usize = 16;

fn entry_open_cmp(domain: &Vic3Node, a: &RankedBagEntry, b: &RankedBagEntry) -> Ordering {
    let open_key =
        |entry: &RankedBagEntry| pea_successor_open_delta(domain, entry).unwrap_or(u32::MAX);
    open_key(a)
        .cmp(&open_key(b))
        .then_with(|| a.tie.cmp(&b.tie))
}

/// Partition so the best `k` candidates are in `bag[..k]` (sorted); the rest
/// stay unordered with `bag[k]` equal to the best deferred score when present.
fn select_top_k_by<F>(bag: &mut [RankedBagEntry], k: usize, mut cmp: F)
where
    F: FnMut(&RankedBagEntry, &RankedBagEntry) -> Ordering,
{
    if bag.is_empty() || k == 0 {
        return;
    }
    let k = k.min(bag.len());
    if bag.len() > k {
        bag.select_nth_unstable_by(k, |a, b| cmp(a, b));
        bag[..k].sort_unstable_by(|a, b| cmp(a, b));
    } else {
        bag.sort_unstable_by(|a, b| cmp(a, b));
    }
}

fn select_top_k(domain: &Vic3Node, bag: &mut [RankedBagEntry], k: usize) {
    select_top_k_by(bag, k, |a, b| entry_open_cmp(domain, a, b));
}

/// Open-set \(f - g\) for one bag row after speculative apply: `edge + h(child)`.
///
/// Ready [`SearchNode::heuristic`] and the resume cursor must use this same
/// scale so A* pops every emitted sibling before the deferred cursor.
pub(crate) fn pea_successor_open_delta(domain: &Vic3Node, entry: &RankedBagEntry) -> Option<u32> {
    let (child, edge) = bag_rank::emit_child(domain, entry)?;
    Some(edge.saturating_add(child.heuristic()))
}

/// Violation of the PEA* partial-expand open-set ordering invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(any(test, debug_assertions))]
pub(crate) struct PeaExpandInvariantViolation {
    pub resume_open_delta: u32,
    pub max_emitted_open_delta: u32,
    pub emitted_count: usize,
}

/// After one beam expand, the resume cursor must not outrank any emitted sibling.
///
/// With shared parent \(g\): \(\min_{deferred}(f-g) \ge \max_{emitted}(f-g)\).
#[cfg(any(test, debug_assertions))]
pub(crate) fn pea_partial_expand_invariant(
    succs: &[(PeaNode, u32)],
) -> Result<(), PeaExpandInvariantViolation> {
    let mut emitted_open = Vec::new();
    let mut resume_open = None;
    for (node, edge) in succs {
        match &node.inner {
            PeaInner::Ready(_) => emitted_open.push(edge.saturating_add(node.heuristic())),
            PeaInner::Expanding { .. } => {
                debug_assert_eq!(*edge, 0, "resume self-edge must be 0-cost");
                debug_assert!(
                    resume_open.is_none(),
                    "at most one resume cursor per expand"
                );
                resume_open = Some(node.heuristic());
            }
        }
    }
    let Some(resume_open_delta) = resume_open else {
        return Ok(());
    };
    let Some(&max_emitted_open_delta) = emitted_open.iter().max() else {
        return Ok(());
    };
    if resume_open_delta >= max_emitted_open_delta {
        Ok(())
    } else {
        Err(PeaExpandInvariantViolation {
            resume_open_delta,
            max_emitted_open_delta,
            emitted_count: emitted_open.len(),
        })
    }
}

#[cfg(debug_assertions)]
fn debug_assert_pea_beam_invariant(
    domain: &Vic3Node,
    succs: &[(PeaNode, u32)],
    deferred: &[RankedBagEntry],
) {
    if let Err(violation) = pea_partial_expand_invariant(succs) {
        debug_assert!(
            false,
            "PEA* resume open delta {} is ahead of emitted sibling max {} \
             (emitted={} fp={:016x})",
            violation.resume_open_delta,
            violation.max_emitted_open_delta,
            violation.emitted_count,
            domain.fingerprint(),
        );
    }
    let min_deferred_open = deferred
        .iter()
        .filter_map(|entry| pea_successor_open_delta(domain, entry))
        .min();
    if let Some((resume, 0)) = succs
        .iter()
        .find(|(node, edge)| matches!(node.inner, PeaInner::Expanding { .. }) && *edge == 0)
    {
        let resume_h = resume.heuristic();
        if let Some(min_deferred) = min_deferred_open {
            debug_assert_eq!(
                resume_h,
                min_deferred,
                "Expanding heuristic must equal best deferred open delta \
                 (resume_h={resume_h} min_deferred={min_deferred} fp={:016x})",
                domain.fingerprint(),
            );
        }
    }
}

/// PEA* search node: domain [`Vic3Node`] or an expansion cursor over a
/// country-wide candidate bag for that domain state.
///
/// Identity is [`PeaInner`]'s Hash/Eq (Ready node, or Expanding domain+emitted);
/// the candidate list is ignored.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PeaNode {
    inner: PeaInner,
}

/// Expanding identity is `(domain, emitted)` only — `candidates` is ignored.
#[derive(Clone, Debug, Derivative)]
#[derivative(PartialEq, Eq, Hash)]
enum PeaInner {
    Ready(Vic3Node),
    /// Resume handle: `candidates` is the **remaining** deferred bag after
    /// prior beams. `emitted` counts how many successors this expand has
    /// already pushed (Hash/Eq). `Rc` makes pathfinding's required [`Clone`]
    /// cheap until `to_vec` on resume.
    Expanding {
        domain: Vic3Node,
        #[derivative(PartialEq = "ignore", Hash = "ignore")]
        candidates: Rc<[RankedBagEntry]>,
        emitted: usize,
    },
}

impl PeaNode {
    /// Wrap a Vic3 root (or any domain node) as a PEA* ready node.
    pub fn ready(node: Vic3Node) -> Self {
        Self {
            inner: PeaInner::Ready(node),
        }
    }

    /// Domain planning node (fingerprint / sim state).
    pub fn domain(&self) -> &Vic3Node {
        match &self.inner {
            PeaInner::Ready(n) => n,
            PeaInner::Expanding { domain, .. } => domain,
        }
    }

    /// Select top‑K from `bag`, emit applied children, defer the rest.
    fn emit_beam(
        domain: &Vic3Node,
        mut bag: Vec<RankedBagEntry>,
        already_emitted: usize,
    ) -> Vec<(Self, u32)> {
        let beam = DEFAULT_PEA_BEAM.max(1);
        if bag.is_empty() {
            return Vec::new();
        }
        select_top_k(domain, &mut bag, beam);
        let take = beam.min(bag.len());
        let deferred_min_cheap = (take < bag.len()).then(|| bag[take].cheap_rank_key);

        let mut out = Vec::with_capacity(take.saturating_add(1));
        for entry in &bag[..take] {
            let Some((child, edge)) = bag_rank::emit_child(domain, entry) else {
                continue;
            };
            let emit_key = bag_rank::emit_rank_key(domain, entry, &child);
            if let Some(mismatch) = bag_rank::emit_deferred_cheap_mismatch(
                entry.cheap_rank_key,
                emit_key,
                deferred_min_cheap,
            ) {
                tracing::warn!(
                    target: "vic3_planning::pea",
                    cheap_rank_key = mismatch.cheap_rank_key,
                    emit_rank_key = mismatch.emit_rank_key,
                    deferred_min_cheap = mismatch.deferred_min_cheap,
                    gdp = child.state().gdp,
                    gdp_for_rates = child.gdp_for_rates(),
                    action = ?entry.action,
                    "PEA beam emit rank exceeds best deferred cheap rank \
                     (cheap under-ranked a deferred rival; see docs/planning-search.md)"
                );
            }
            out.push((Self::ready(child), edge));
        }
        let deferred = take < bag.len();
        domain.note_beam_emit(out.len(), deferred);
        if deferred {
            let deferred_slice = &bag[take..];
            out.push((
                Self {
                    inner: PeaInner::Expanding {
                        domain: domain.clone(),
                        candidates: Rc::from(deferred_slice.to_vec()),
                        emitted: already_emitted.saturating_add(take),
                    },
                },
                0,
            ));
            #[cfg(debug_assertions)]
            debug_assert_pea_beam_invariant(domain, &out, deferred_slice);
        }
        out
    }
}

impl SearchNode for PeaNode {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, Self::Cost)> {
        match &self.inner {
            PeaInner::Ready(domain) => {
                let bag = bag_rank::cheap_rank_bag(domain);
                domain.note_pea_ready(bag.len());
                super::astar_trace::on_expand("pea-ready", || {
                    let (fp_dups, fp_uniques) = domain.fingerprint_dup_stats();
                    format!(
                        "fp={:016x} gdp={:.0} h={} candidates={} beam={} fp_dups={} fp_uniques={}",
                        domain.fingerprint(),
                        domain.state().gdp,
                        domain.heuristic(),
                        bag.len(),
                        DEFAULT_PEA_BEAM,
                        fp_dups,
                        fp_uniques,
                    )
                });
                if bag.is_empty() {
                    return Vec::new();
                }
                Self::emit_beam(domain, bag, 0)
            }
            PeaInner::Expanding {
                domain,
                candidates,
                emitted,
            } => {
                domain.note_pea_resume();
                super::astar_trace::on_expand("pea-resume", || {
                    let (fp_dups, fp_uniques) = domain.fingerprint_dup_stats();
                    format!(
                        "fp={:016x} gdp={:.0} emitted={} remaining={} beam={} fp_dups={} fp_uniques={}",
                        domain.fingerprint(),
                        domain.state().gdp,
                        emitted,
                        candidates.len(),
                        DEFAULT_PEA_BEAM,
                        fp_dups,
                        fp_uniques,
                    )
                });
                Self::emit_beam(domain, candidates.to_vec(), *emitted)
            }
        }
    }

    fn is_goal(&self) -> bool {
        match &self.inner {
            PeaInner::Ready(n) => n.is_goal(),
            PeaInner::Expanding { .. } => false,
        }
    }

    fn heuristic(&self) -> Self::Cost {
        match &self.inner {
            PeaInner::Ready(n) => n.heuristic(),
            PeaInner::Expanding {
                domain, candidates, ..
            } => candidates
                .iter()
                .filter_map(|entry| pea_successor_open_delta(domain, entry))
                .min()
                .unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::plan::pathfinding::shortest_path;
    use crate::sim::{Action, EconomyContext, SimConfig};
    use crate::world::{PlanningParts, PlanningState};
    use proptest::prelude::*;
    use rust_advanced_heaps::pairing::PairingHeap;
    use vic3_defs::{BuildingType, GameDefs, Good, GoodId, GoodsVec, ProductionMethod};
    use vic3_prices::{SolveOpts, World, WorldCountry, WorldState};

    /// Check partial-expand ordering on `succs` and, when present, one resume re-expand.
    fn assert_partial_expand_invariant_on_successors(succs: &[(PeaNode, u32)]) {
        pea_partial_expand_invariant(succs)
            .unwrap_or_else(|v| panic!("PEA* resume ahead of emitted sibling: {v:?}"));
        if let Some((resume, _)) = succs
            .iter()
            .find(|(n, _)| matches!(n.inner, PeaInner::Expanding { .. }))
        {
            assert_partial_expand_invariant_on_successors(&resume.successors());
        }
    }

    /// GDP root with `state_count` owned states and `building_kinds` greenfield output types.
    ///
    /// Branching is roughly `state_count * building_kinds` queue-building successors.
    fn gdp_many_placements_fixture(state_count: usize, building_kinds: usize) -> Vic3Node {
        assert!(state_count >= 1);
        assert!(building_kinds >= 1);

        let mut defs = GameDefs {
            price_range: 0.75,
            ..GameDefs::default()
        };
        register_good_fixture(&mut defs, "wood", 20.0);

        let building_ids: Vec<String> = (0..building_kinds)
            .map(|i| format!("building_output_{i}"))
            .collect();
        for (i, id) in building_ids.iter().enumerate() {
            register_output_building_fixture(
                &mut defs,
                id,
                "wood",
                10.0 + i as f64,
                30.0 + i as f64,
            );
        }
        register_construction_sector_fixture(&mut defs, 5.0, 10.0);
        defs.rebuild_building_types_order();

        let states: Vec<WorldState> = (1..=state_count as u32)
            .map(|id| WorldState {
                id,
                country: Some(1),
                ..WorldState::default()
            })
            .collect();
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states,
            frozen_buy: GoodsVec::from_vec(vec![15.0]),
            ..World::default()
        };
        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let baseline = vic3_prices::solve(
            &economy.base_world,
            &economy.defs,
            economy.solve_opts.clone(),
        );
        let gdp_curr = baseline.buildings.iter().map(|b| b.revenue).sum::<f64>();
        let target = gdp_curr + 500.0;
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            gdp: gdp_curr,
            construction_points_per_day: 5.0,
            good_prices: vec![("wood".into(), baseline.goods[0].price)],
            ..PlanningParts::default()
        });
        let config = SimConfig {
            construction_days: 30,
            default_construction_cost: 30,
            max_added_levels_per_type: 2,
            ..SimConfig::default()
        };
        Vic3Node::new(
            state,
            compile(&format!("gdp >= {target}")).unwrap(),
            config,
            economy,
        )
    }

    fn register_good_fixture(defs: &mut GameDefs, id: &str, base_price: f64) {
        let idx = GoodId::from_usize(defs.goods_order.len());
        defs.goods_order.push(id.into());
        defs.goods.insert(
            id.into(),
            Good {
                name: id.into(),
                base_price,
                traded_quantity: 10.0,
                texture: None,
            },
        );
        let _ = idx;
    }

    fn register_output_building_fixture(
        defs: &mut GameDefs,
        building: &str,
        good_id: &str,
        output_qty: f64,
        required_construction: f64,
    ) {
        let good = defs.index_of(good_id).expect("good registered");
        let pmg = format!("pmg_{building}");
        let pm = format!("pm_{building}");
        defs.building_types.insert(
            building.into(),
            BuildingType {
                name: building.into(),
                group: None,
                city_type: None,
                production_method_groups: vec![pmg.clone()],
                required_construction: Some(required_construction),
            },
        );
        if !defs.building_types_order.iter().any(|id| id == building) {
            defs.building_types_order.push(building.into());
        }
        defs.production_method_groups.insert(pmg, vec![pm.clone()]);
        defs.production_methods.insert(
            pm.clone(),
            ProductionMethod {
                name: pm.clone(),
                outputs: vec![(good, output_qty)],
                ..ProductionMethod::default()
            },
        );
    }

    fn register_construction_sector_fixture(
        defs: &mut GameDefs,
        construction_add: f64,
        required_construction: f64,
    ) {
        use crate::construction::BUILDING_CONSTRUCTION_SECTOR;
        let pmg = "pmg_base_building_construction_sector";
        let pm = "pm_iron_frame_buildings";
        defs.building_types.insert(
            BUILDING_CONSTRUCTION_SECTOR.into(),
            BuildingType {
                name: BUILDING_CONSTRUCTION_SECTOR.into(),
                group: None,
                city_type: None,
                production_method_groups: vec![pmg.into()],
                required_construction: Some(required_construction),
            },
        );
        if !defs
            .building_types_order
            .iter()
            .any(|id| id == BUILDING_CONSTRUCTION_SECTOR)
        {
            defs.building_types_order
                .push(BUILDING_CONSTRUCTION_SECTOR.into());
        }
        defs.production_method_groups
            .insert(pmg.into(), vec![pm.into()]);
        defs.production_methods.insert(
            pm.into(),
            ProductionMethod {
                name: pm.into(),
                country_construction_add: Some(construction_add),
                ..ProductionMethod::default()
            },
        );
    }

    #[test]
    fn pea_partial_expand_resume_not_ahead_of_emitted_siblings() {
        let root = gdp_many_placements_fixture(6, 4);
        let branch = root.sim_successors().len();
        assert!(
            branch > DEFAULT_PEA_BEAM,
            "fixture must defer (branch={branch}, beam={DEFAULT_PEA_BEAM})"
        );
        let succs = PeaNode::ready(root).successors();
        assert!(
            succs
                .iter()
                .any(|(n, _)| matches!(n.inner, PeaInner::Expanding { .. })),
            "expected Expanding cursor when branch > beam"
        );
        assert_partial_expand_invariant_on_successors(&succs);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Universal PEA* property: resume open priority is never ahead of any
        /// emitted sibling from the same partial expand (ready or resume).
        #[test]
        fn pea_partial_expand_resume_behind_emitted(
            state_count in 3usize..=12usize,
            building_kinds in 2usize..=6usize,
        ) {
            prop_assume!(state_count.saturating_mul(building_kinds) > DEFAULT_PEA_BEAM);
            let root = gdp_many_placements_fixture(state_count, building_kinds);
            prop_assume!(root.sim_successors().len() > DEFAULT_PEA_BEAM);
            let succs = PeaNode::ready(root).successors();
            prop_assume!(succs.iter().any(|(n, _)| matches!(n.inner, PeaInner::Expanding { .. })));
            assert_partial_expand_invariant_on_successors(&succs);
        }
    }

    #[test]
    fn select_top_k_puts_best_prefix_and_bound() {
        fn cheap_cmp(a: &RankedBagEntry, b: &RankedBagEntry) -> Ordering {
            a.cheap_rank_key
                .cmp(&b.cheap_rank_key)
                .then_with(|| a.tie.cmp(&b.tie))
        }
        let mut bag: Vec<RankedBagEntry> = [30u32, 10, 40, 20, 50, 15]
            .into_iter()
            .enumerate()
            .map(|(i, key)| RankedBagEntry {
                action: Action::QueueTech {
                    tech: format!("t{i}"),
                },
                days: 0,
                cheap_rank_key: key,
                tie: i as u64,
            })
            .collect();
        select_top_k_by(&mut bag, 3, cheap_cmp);
        let prefix: Vec<u32> = bag[..3].iter().map(|c| c.cheap_rank_key).collect();
        assert_eq!(prefix, vec![10, 15, 20]);
        let deferred_min = bag[3].cheap_rank_key;
        assert_eq!(deferred_min, 30);
        assert!(bag[4..].iter().all(|c| c.cheap_rank_key >= deferred_min));
    }

    #[test]
    fn pea_matches_vic3_day_cost_on_research_fixture() {
        let root = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                ..PlanningParts::default()
            }),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
            EconomyContext::empty(),
        );
        let (_, vic3_cost) = shortest_path::<_, PairingHeap<_, _>>(&root).unwrap();
        let (_, pea_cost) = shortest_path::<_, PairingHeap<_, _>>(&PeaNode::ready(root)).unwrap();
        assert_eq!(pea_cost, vic3_cost);
        assert_eq!(pea_cost, 365);
    }

    #[test]
    fn spotcheck_pea_agrees_with_full_astar_on_branching_fixtures() {
        use crate::military::{ModeledMilBuilding, UnitCombatStats, BUILDING_BARRACKS};
        use crate::plan::plan;

        let cases: Vec<(&str, Vic3Node)> = vec![
            (
                "research",
                Vic3Node::new(
                    PlanningState::from_parts(PlanningParts {
                        country: "GER".into(),
                        ..PlanningParts::default()
                    }),
                    compile("research(tech=nitroglycerin)").unwrap(),
                    SimConfig::default(),
                    EconomyContext::empty(),
                ),
            ),
            (
                "research_and",
                Vic3Node::new(
                    PlanningState::from_parts(PlanningParts {
                        country: "GER".into(),
                        ..PlanningParts::default()
                    }),
                    compile("has_tech(railways) && has_tech(nitroglycerin)").unwrap(),
                    SimConfig::default(),
                    EconomyContext::empty(),
                ),
            ),
            (
                "interest",
                Vic3Node::new(
                    PlanningState::default(),
                    compile("interest_in(region=region_western_europe)").unwrap(),
                    SimConfig {
                        interest_days: 25,
                        ..SimConfig::default()
                    },
                    EconomyContext::empty(),
                ),
            ),
            (
                "law",
                Vic3Node::new(
                    PlanningState::default(),
                    compile("has_law(law_homesteading)").unwrap(),
                    SimConfig::default(),
                    EconomyContext::empty(),
                ),
            ),
            (
                "declare_war",
                Vic3Node::new(
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
                ),
            ),
            ("army", {
                let per = UnitCombatStats::army_default().full_power_projection();
                let levels = (100.0 / per).ceil();
                Vic3Node::new(
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
                )
            }),
            (
                "solvent",
                Vic3Node::new(
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
                ),
            ),
        ];

        for (name, root) in cases {
            let branch = root.sim_successors().len();
            let (_, vic3_cost) = shortest_path::<_, PairingHeap<_, _>>(&root)
                .unwrap_or_else(|| panic!("{name}: full A* unreachable"));
            let pea_root = PeaNode::ready(root.clone());
            let pea_branch = pea_root.successors().len();
            let (_, pea_cost) = shortest_path::<_, PairingHeap<_, _>>(&pea_root)
                .unwrap_or_else(|| panic!("{name}: PEA* unreachable"));
            assert_eq!(
                pea_cost, vic3_cost,
                "{name}: PEA day_cost {pea_cost} != full A* {vic3_cost} (sim_branch={branch}, pea_first_expand={pea_branch})"
            );

            let via_plan = plan(
                root.state().clone(),
                root.goal().clone(),
                root.config(),
                EconomyContext::empty(),
                10_000,
                0.0,
                vec![],
            )
            .unwrap_or_else(|e| panic!("{name}: plan() failed: {e}"));
            assert_eq!(
                via_plan.day_cost, vic3_cost,
                "{name}: plan() day_cost {} != full A* {vic3_cost}",
                via_plan.day_cost
            );
            eprintln!(
                "spotcheck ok {name}: days={vic3_cost} sim_branch={branch} pea_inserts={pea_branch} actions={}",
                via_plan.actions.len()
            );
        }
    }

    #[test]
    fn emit_beam_defers_past_fixed_width() {
        let root = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                ..PlanningParts::default()
            }),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
            EconomyContext::empty(),
        );
        let pea = PeaNode::ready(root);
        let succs = pea.successors();
        assert!(succs
            .iter()
            .all(|(n, _)| matches!(n.inner, PeaInner::Ready(_))));
        assert!(!succs.is_empty());
    }

    #[test]
    fn pea_emit_sets_gdp_for_rates_from_speculative_delta() {
        use crate::test_support::{ger_state, logging_and_cs_economy};
        use vic3_prices::solve;

        let mini = logging_and_cs_economy();
        let baseline = solve(
            &mini.economy.base_world,
            &mini.economy.defs,
            mini.economy.solve_opts.clone(),
        );
        let logging_id = mini
            .economy
            .defs
            .building_index_of("building_logging_camp")
            .expect("logging camp type");
        let bumped = solve(
            &mini.economy.base_world.with_extra_levels(logging_id, 1),
            &mini.economy.defs,
            mini.economy.solve_opts.clone(),
        );
        let gdp_curr = baseline.buildings[0].revenue;
        let expected_completed_gdp = bumped.buildings[0].revenue;
        assert!(
            expected_completed_gdp > gdp_curr,
            "fixture must raise GDP on complete"
        );

        let state = ger_state()
            .gdp(gdp_curr)
            .points(5.0)
            .wood_price(baseline.goods[0].price)
            .get();

        let target = (gdp_curr + expected_completed_gdp) / 2.0;
        let config = SimConfig {
            construction_days: 30,
            default_construction_cost: 30,
            max_added_levels_per_type: 2,
            ..mini.config
        };
        let root = Vic3Node::new(
            state,
            compile(&format!("gdp >= {target}")).unwrap(),
            config,
            mini.economy,
        );

        let action = Action::QueueBuildingLevel {
            building_type_name: "building_logging_camp".into(),
            state_id: 1,
        };
        let completed = crate::sim::speculative_completed_state(
            root.state(),
            &action,
            root.economy(),
            root.config(),
        )
        .expect("speculative complete for logging enqueue");
        assert!(
            (completed.gdp - expected_completed_gdp).abs() < 1e-6,
            "completed.gdp={} expected={}",
            completed.gdp,
            expected_completed_gdp
        );

        let emit_gdp_delta = completed.gdp - gdp_curr;
        let anticipated = gdp_curr + emit_gdp_delta;
        let node = root.clone().with_gdp_for_rates(anticipated);
        assert!((node.gdp_for_rates() - completed.gdp).abs() < 1e-9);
        assert!((node.gdp_for_rates() - gdp_curr).abs() > 1e-6);

        let succs = PeaNode::ready(root).successors();
        let child = succs
            .iter()
            .find_map(|(n, _)| {
                if !matches!(n.inner, PeaInner::Ready(_)) {
                    return None;
                }
                let d = n.domain();
                let queued = d.state().constructions.iter().any(|row| {
                    row.building_type_name == "building_logging_camp" && row.state_id == Some(1)
                });
                queued.then_some(d)
            })
            .expect("expected Ready PEA child for QueueBuildingLevel logging camp");

        assert!(
            (child.state().gdp - gdp_curr).abs() < 1e-6,
            "search successor GDP must stay enqueue-only (state_t / pre-complete), got {}",
            child.state().gdp
        );
        assert!(
            (child.gdp_for_rates() - anticipated).abs() < 1e-6,
            "gdp_for_rates={} expected anticipated={}",
            child.gdp_for_rates(),
            anticipated
        );
        assert!(
            (child.gdp_for_rates() - child.state().gdp).abs() > 1e-6,
            "gdp_for_rates must differ from enqueue-only state.gdp"
        );
    }

    /// Live Prussia GDP bump: PEA* day cost must match full A*.
    #[test]
    #[ignore = "set VIC3_SAVE and VIC3_DEFS for live Prussia GDP spot-check"]
    fn spotcheck_live_save_gdp_pea_vs_full_astar() {
        use std::time::Instant;
        use vic3_load::{empty_tokens, load_path_world, load_tokens_path};
        use vic3_prices::{solve, SolveOpts, World};

        let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE");
        let defs = {
            let path = std::env::var("VIC3_DEFS").expect("VIC3_DEFS");
            vic3_defs::decode_blob(&std::fs::read(path).expect("defs blob")).expect("decode")
        };
        let tokens = match std::env::var("VIC3_TOKENS") {
            Ok(path) => load_tokens_path(path).expect("tokens"),
            Err(_) => empty_tokens(),
        };
        let save = load_path_world(&save_path, tokens).expect("load save");
        let world = World::from_save(&save, &defs);
        let country = world.player_country_tag().expect("player tag").to_string();
        let prices = solve(&world, &defs, SolveOpts::default());
        let state =
            PlanningState::from_world_with_prices(&world, &country, &prices, &defs).expect("state");
        let current_gdp = state.gdp;
        let target = (current_gdp * 1.005).max(current_gdp + 500.0);
        let goal_src = format!("gdp >= {target}");
        eprintln!("live: country={country} gdp={current_gdp} target={target}");

        let economy = EconomyContext::new(world, defs, SolveOpts::default());
        let root = Vic3Node::new_with_economy(
            state,
            compile(&goal_src).expect("compile"),
            SimConfig::default(),
            economy,
        );
        let branch = root.sim_successors().len();
        let pea_inserts = PeaNode::ready(root.clone()).successors().len();
        eprintln!("live: sim_branch={branch} pea_first_expand_inserts={pea_inserts} (beam={DEFAULT_PEA_BEAM})");
        assert!(branch > 0, "expected GDP successors; got 0");

        let t0 = Instant::now();
        let (_, vic3_cost) =
            shortest_path::<_, PairingHeap<_, _>>(&root).expect("full A* GDP bump");
        let vic3_ms = t0.elapsed().as_millis();

        let t1 = Instant::now();
        let (_, pea_cost) =
            shortest_path::<_, PairingHeap<_, _>>(&PeaNode::ready(root)).expect("PEA* GDP bump");
        let pea_ms = t1.elapsed().as_millis();

        eprintln!("live: full_A*={vic3_cost}d ({vic3_ms}ms) PEA*={pea_cost}d ({pea_ms}ms)");
        assert_eq!(pea_cost, vic3_cost);
    }
}
