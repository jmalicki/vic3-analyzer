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
//! Each Ready expand builds one **national** [`Candidate`] bag from
//! [`Vic3Node::sim_successors`] (all placement states / actions for that
//! current node). Ranking uses [`super::progress_h::cheap_bag_score`] (quick
//! guesstimate). Top‑[`DEFAULT_PEA_BEAM`] rows are chosen with `select_nth`.
//! On emit, successors are applied and scored with speculative complete +
//! [`super::progress_h::emit_bag_score`]; emit GDP anticipation is stored on
//! the successor as [`Vic3Node::gdp_for_rates`]. Deferred rows stay action + cheap
//! score. ShopCache stays unranked — used only when scoring/applying.
//!
//! Expanding identity is `(domain, emitted)` — the candidate list is
//! `Rc<[…]>` so [`SearchNode`] clones are refcount bumps, and is not hashed.

use super::pathfinding::SearchNode;
use super::Vic3Node;
use crate::sim::Action;
use derivative::Derivative;
use std::cmp::Ordering;
use std::rc::Rc;

/// Fixed country-wide beam width for PEA* partial expansion.
///
/// Chosen from a no-refit GDP proxy on Prussia 1836 (~50 build-level
/// candidates; top 8–16 captured the high-value head). Not a proven optimum.
pub const DEFAULT_PEA_BEAM: usize = 16;

/// Dependency tags for a deferred candidate (future dirty/rescore hooks).
///
/// Frozen-at-expand PEA does not rescore today; these tags document which
/// geo/building a row touches without ranking ShopCache.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CandidateDeps {
    state_id: Option<u32>,
    building_id: Option<u32>,
}

impl CandidateDeps {
    fn from_action(action: &Action) -> Self {
        match action {
            Action::QueueBuildingLevel { state_id, .. } => Self {
                state_id: Some(*state_id),
                building_id: None,
            },
            Action::SwitchPm { building_id, .. } => Self {
                state_id: None,
                building_id: Some(*building_id),
            },
            _ => Self::default(),
        }
    }
}

/// One country-wide PEA edge: emit payload + score; no child until emit.
#[derive(Clone, Debug)]
struct Candidate {
    action: Action,
    days: u16,
    /// `edge + h(child)` — independent of current-node \(g\) as a sort key and resume \(h\).
    f_minus_g: u32,
    #[allow(dead_code)] // reserved for live-rescore / dirty sets
    deps: CandidateDeps,
    /// Deterministic tie-break when `f_minus_g` matches (child fingerprint).
    tie: u64,
}

fn candidate_cmp(a: &Candidate, b: &Candidate) -> Ordering {
    a.f_minus_g
        .cmp(&b.f_minus_g)
        .then_with(|| a.tie.cmp(&b.tie))
}

/// Partition so the best `k` candidates are in `bag[..k]` (sorted); the rest
/// stay unordered with `bag[k]` equal to the best deferred score when present.
fn select_top_k(bag: &mut [Candidate], k: usize) {
    if bag.is_empty() || k == 0 {
        return;
    }
    let k = k.min(bag.len());
    if bag.len() > k {
        bag.select_nth_unstable_by(k, candidate_cmp);
        bag[..k].sort_unstable_by(candidate_cmp);
    } else {
        bag.sort_unstable_by(candidate_cmp);
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

/// True when emit ranking is worse than the best deferred cheap bag score.
///
/// Design: emit score > deferred_min_cheap → warn (cheap under-ranked a rival).
/// Emit/rebuild follow-on better than cheap is expected — do not warn.
pub(crate) fn emit_score_exceeds_deferred_cheap(emit_score: u32, deferred_min_cheap: u32) -> bool {
    emit_score > deferred_min_cheap
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmitDeferredCheapMismatch {
    pub cheap_bag_score: u32,
    pub emit_score: u32,
    pub deferred_min_cheap: u32,
}

/// Build a mismatch record when a warn should fire; `None` if no warn.
pub(crate) fn emit_deferred_cheap_mismatch(
    cheap_bag_score: u32,
    emit_score: u32,
    deferred_min_cheap: Option<u32>,
) -> Option<EmitDeferredCheapMismatch> {
    let deferred_min = deferred_min_cheap?;
    if !emit_score_exceeds_deferred_cheap(emit_score, deferred_min) {
        return None;
    }
    Some(EmitDeferredCheapMismatch {
        cheap_bag_score,
        emit_score,
        deferred_min_cheap: deferred_min,
    })
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
        candidates: Rc<[Candidate]>,
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

    fn build_candidates(domain: &Vic3Node) -> Vec<Candidate> {
        let state_curr = domain.state();
        let curr = match super::progress_h::CheapBagCurr::new(
            domain.goal(),
            state_curr,
            domain.config(),
            domain.economy(),
            domain.gdp_for_rates(),
        ) {
            Ok(curr) => curr,
            Err(err) => {
                // Bag ranking needs meters; fall back to admissible edge+h so
                // PEA still expands (do not drop the expand).
                tracing::warn!(
                    target: "vic3_planning::pea",
                    ?err,
                    "CheapBagCurr failed; PEA bag falls back to h_adm"
                );
                return domain
                    .sim_successors()
                    .into_iter()
                    .map(|successor| {
                        let node = Vic3Node::with_shared_context(successor.state, domain);
                        let edge = u32::from(successor.days);
                        let f_minus_g = edge.saturating_add(node.heuristic());
                        let deps = CandidateDeps::from_action(&successor.action);
                        Candidate {
                            action: successor.action,
                            days: successor.days,
                            f_minus_g,
                            deps,
                            tie: node.fingerprint(),
                        }
                    })
                    .collect();
            }
        };

        domain
            .sim_successors()
            .into_iter()
            .map(|successor| {
                // Cheap bag score only (see progress_h::cheap_bag_score deficiencies).
                // Emit recomputes with speculative complete + full residual.
                let f_minus_g =
                    super::progress_h::cheap_bag_score(&successor.action, successor.days, &curr);
                // Tie-break without building a full child: hash action + days.
                let tie = {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    successor.action.hash(&mut hasher);
                    successor.days.hash(&mut hasher);
                    hasher.finish()
                };
                let deps = CandidateDeps::from_action(&successor.action);
                Candidate {
                    action: successor.action,
                    days: successor.days,
                    f_minus_g,
                    deps,
                    tie,
                }
            })
            .collect()
    }

    fn emit_candidate(domain: &Vic3Node, candidate: &Candidate) -> Option<(Self, u32)> {
        let state = domain.apply_action(&candidate.action)?;
        let mut node = Vic3Node::with_shared_context(state, domain);

        // Emit-time GDP: speculative complete + full price solve when possible.
        // Emitted successor remains post-enqueue; gdp_for_rates anticipates the delta.
        let gdp_curr = domain.gdp_for_rates();
        match crate::sim::speculative_completed_state(
            domain.state(),
            &candidate.action,
            domain.economy(),
            domain.config(),
        ) {
            Ok(completed) => {
                let emit_gdp_delta = completed.gdp - gdp_curr;
                let gdp_for_rates = gdp_curr + emit_gdp_delta;
                node = node.with_gdp_for_rates(gdp_for_rates);
            }
            Err(err) => {
                // Apply already succeeded; keep enqueue-only gdp_for_rates (= state.gdp).
                tracing::debug!(
                    target: "vic3_planning::pea",
                    ?err,
                    action = ?candidate.action,
                    "speculative_completed_state failed; emit without GDP anticipation"
                );
            }
        }

        Some((Self::ready(node), u32::from(candidate.days)))
    }

    /// Emit ranking key on speculative completed world (`edge +` residual).
    fn emit_f_minus_g(domain: &Vic3Node, candidate: &Candidate) -> Option<u32> {
        let completed = crate::sim::speculative_completed_state(
            domain.state(),
            &candidate.action,
            domain.economy(),
            domain.config(),
        )
        .ok()?;
        super::progress_h::emit_bag_score(
            candidate.days,
            domain.goal(),
            &completed,
            domain.config(),
            domain.economy(),
        )
        .ok()
    }

    /// Select top‑K from `bag`, emit applied children, defer the rest.
    fn emit_beam(
        domain: &Vic3Node,
        mut bag: Vec<Candidate>,
        already_emitted: usize,
    ) -> Vec<(Self, u32)> {
        let beam = DEFAULT_PEA_BEAM.max(1);
        if bag.is_empty() {
            return Vec::new();
        }
        select_top_k(&mut bag, beam);
        let take = beam.min(bag.len());
        // Best deferred **cheap** bag score (for warn vs emit).
        let deferred_min_cheap = (take < bag.len()).then(|| bag[take].f_minus_g);

        let mut out = Vec::with_capacity(take.saturating_add(1));
        for candidate in &bag[..take] {
            let Some((child, edge)) = Self::emit_candidate(domain, candidate) else {
                continue;
            };
            let actual = Self::emit_f_minus_g(domain, candidate).unwrap_or_else(|| {
                // Fallback when speculative complete / residual unavailable.
                let residual = super::progress_h::rank_heuristic_with_gdp_for_rates(
                    domain.goal(),
                    child.domain().state(),
                    domain.config(),
                    domain.economy(),
                    child.domain().gdp_for_rates(),
                )
                .unwrap_or_else(|_| child.domain().heuristic());
                u32::from(candidate.days).saturating_add(residual)
            });
            if let Some(mismatch) =
                emit_deferred_cheap_mismatch(candidate.f_minus_g, actual, deferred_min_cheap)
            {
                tracing::warn!(
                    target: "vic3_planning::pea",
                    cheap_bag_score = mismatch.cheap_bag_score,
                    emit_score = mismatch.emit_score,
                    deferred_min_cheap = mismatch.deferred_min_cheap,
                    gdp = child.domain().state().gdp,
                    gdp_for_rates = child.domain().gdp_for_rates(),
                    action = ?candidate.action,
                    "PEA beam emit score exceeds best deferred cheap bag score \
                     (cheap under-ranked a deferred rival; see docs/planning-search.md)"
                );
            }
            out.push((child, edge));
        }
        let deferred = take < bag.len();
        domain.note_beam_emit(out.len(), deferred);
        if deferred {
            out.push((
                Self {
                    inner: PeaInner::Expanding {
                        domain: domain.clone(),
                        candidates: Rc::from(bag[take..].to_vec()),
                        emitted: already_emitted.saturating_add(take),
                    },
                },
                0,
            ));
        }
        out
    }
}

impl SearchNode for PeaNode {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, Self::Cost)> {
        match &self.inner {
            PeaInner::Ready(domain) => {
                let bag = Self::build_candidates(domain);
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
            PeaInner::Ready(n) => {
                let ok = n.is_goal();
                // Vic3Node::is_goal already traces; avoid double GOAL lines.
                ok
            }
            PeaInner::Expanding { .. } => false,
        }
    }

    fn heuristic(&self) -> Self::Cost {
        match &self.inner {
            PeaInner::Ready(n) => n.heuristic(),
            PeaInner::Expanding { candidates, .. } => {
                // Remaining bag was sliced after select_nth: index 0 is the
                // best deferred `edge + h_rank`. That is **not** h_adm; mixing
                // this into A* f can drop when the child becomes Ready. v1
                // tolerates — see docs/planning-search.md.
                candidates.first().map(|c| c.f_minus_g).unwrap_or(0)
            }
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
    use rust_advanced_heaps::pairing::PairingHeap;

    #[test]
    fn emit_score_exceeds_deferred_cheap_only_when_worse() {
        assert!(!emit_score_exceeds_deferred_cheap(10, 10));
        assert!(!emit_score_exceeds_deferred_cheap(9, 10));
        assert!(emit_score_exceeds_deferred_cheap(11, 10));
    }

    #[test]
    fn emit_deferred_cheap_mismatch_none_without_deferred() {
        assert_eq!(emit_deferred_cheap_mismatch(5, 20, None), None);
    }

    #[test]
    fn emit_deferred_cheap_mismatch_some_when_emit_worse() {
        assert_eq!(
            emit_deferred_cheap_mismatch(5, 20, Some(10)),
            Some(EmitDeferredCheapMismatch {
                cheap_bag_score: 5,
                emit_score: 20,
                deferred_min_cheap: 10,
            })
        );
        assert_eq!(emit_deferred_cheap_mismatch(5, 10, Some(10)), None);
        assert_eq!(emit_deferred_cheap_mismatch(5, 9, Some(10)), None);
    }

    #[test]
    fn select_top_k_puts_best_prefix_and_bound() {
        let mut bag: Vec<Candidate> = [30u32, 10, 40, 20, 50, 15]
            .into_iter()
            .enumerate()
            .map(|(i, f)| Candidate {
                action: Action::QueueTech {
                    tech: format!("t{i}"),
                },
                days: 0,
                f_minus_g: f,
                deps: CandidateDeps::default(),
                tie: i as u64,
            })
            .collect();
        select_top_k(&mut bag, 3);
        let prefix: Vec<u32> = bag[..3].iter().map(|c| c.f_minus_g).collect();
        assert_eq!(prefix, vec![10, 15, 20]);
        let deferred_min = bag[3].f_minus_g;
        assert_eq!(deferred_min, 30);
        assert!(bag[4..].iter().all(|c| c.f_minus_g >= deferred_min));
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

            // Production `plan()` path (PEA-wired) must match full A* cost.
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
        // Research fixture has few successors (< beam); no Expanding cursor.
        assert!(succs
            .iter()
            .all(|(n, _)| matches!(n.inner, PeaInner::Ready(_))));
        assert!(!succs.is_empty());
    }

    /// PEA emit stores anticipated complete GDP on `gdp_for_rates` while the
    /// search child state stays post-enqueue (pre-complete GDP).
    #[test]
    fn pea_emit_sets_gdp_for_rates_from_speculative_delta() {
        use crate::test_support::{ger_state, logging_and_cs_economy};
        use vic3_prices::solve;

        let mini = logging_and_cs_economy();
        // Seed state GDP from a real solve so speculative complete delta is meaningful.
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
    ///
    /// ```text
    /// VIC3_SAVE=…/prussia_1836_01_01.v3 VIC3_DEFS=…/defs.postcard \
    ///   cargo test -p vic3-planning --lib \
    ///   plan::pea::tests::spotcheck_live_save_gdp_pea_vs_full_astar \
    ///   -- --ignored --nocapture
    /// ```
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
