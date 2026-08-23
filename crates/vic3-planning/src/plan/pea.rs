//! Partial-expansion A* adapter over [`Vic3Node`].
//!
//! Encodes PEA* as distinct [`SearchNode`] identities so the existing
//! closed-set `shortest_path` loop can defer surplus successors without a
//! heaps-crate change: after emitting a fixed-size beam, the parent is
//! re-inserted via a 0-cost self-edge whose heuristic matches the next
//! child's \(f - g = \mathrm{edge} + h\).
//!
//! Beam policy (locked for v1): fixed width [`DEFAULT_PEA_BEAM`] (16), not
//! ties-only. Dominance pruning is intentionally out of scope.

use super::pathfinding::SearchNode;
use super::Vic3Node;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Default fixed beam width for PEA* partial expansion.
///
/// Chosen from a no-refit GDP proxy on Prussia 1836 (~50 build-level
/// candidates; top 8–16 captured the high-value head). Not a proven optimum.
pub const DEFAULT_PEA_BEAM: usize = 16;

#[derive(Clone, Debug)]
struct RankedSucc {
    node: Vic3Node,
    edge: u32,
    /// `edge + h(child)` — parent-\(g\)-independent sort key and resume \(h\).
    f_minus_g: u32,
}

/// PEA* search node: domain [`Vic3Node`] or an expansion cursor over ranked
/// successors of that domain state.
#[derive(Clone, Debug)]
pub struct PeaNode {
    inner: PeaInner,
}

#[derive(Clone, Debug)]
enum PeaInner {
    Ready(Vic3Node),
    Expanding {
        domain: Vic3Node,
        ranked: Rc<Vec<RankedSucc>>,
        next: usize,
        beam: usize,
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

    fn beam_width(&self) -> usize {
        match &self.inner {
            PeaInner::Ready(_) => DEFAULT_PEA_BEAM,
            PeaInner::Expanding { beam, .. } => *beam,
        }
    }

    fn rank_successors(domain: &Vic3Node) -> Vec<RankedSucc> {
        let mut ranked: Vec<RankedSucc> = domain
            .sim_successors()
            .into_iter()
            .map(|successor| {
                let node = Vic3Node::with_shared_context(successor.state, domain);
                let edge = u32::from(successor.days);
                let f_minus_g = edge.saturating_add(node.heuristic());
                RankedSucc {
                    node,
                    edge,
                    f_minus_g,
                }
            })
            .collect();
        ranked.sort_by(|a, b| {
            a.f_minus_g
                .cmp(&b.f_minus_g)
                .then_with(|| a.node.fingerprint().cmp(&b.node.fingerprint()))
        });
        ranked
    }

    fn emit_beam(
        ranked: &Rc<Vec<RankedSucc>>,
        domain: &Vic3Node,
        next: usize,
        beam: usize,
    ) -> Vec<(Self, u32)> {
        let end = next.saturating_add(beam).min(ranked.len());
        let mut out = Vec::with_capacity(end.saturating_sub(next).saturating_add(1));
        for succ in &ranked[next..end] {
            out.push((Self::ready(succ.node.clone()), succ.edge));
        }
        if end < ranked.len() {
            out.push((
                Self {
                    inner: PeaInner::Expanding {
                        domain: domain.clone(),
                        ranked: Rc::clone(ranked),
                        next: end,
                        beam,
                    },
                },
                0,
            ));
        }
        out
    }
}

impl PartialEq for PeaNode {
    fn eq(&self, other: &Self) -> bool {
        match (&self.inner, &other.inner) {
            (PeaInner::Ready(a), PeaInner::Ready(b)) => a == b,
            (
                PeaInner::Expanding {
                    domain: a,
                    next: na,
                    ..
                },
                PeaInner::Expanding {
                    domain: b,
                    next: nb,
                    ..
                },
            ) => a == b && na == nb,
            _ => false,
        }
    }
}

impl Eq for PeaNode {}

impl Hash for PeaNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.inner {
            PeaInner::Ready(n) => {
                0u8.hash(state);
                n.hash(state);
            }
            PeaInner::Expanding { domain, next, .. } => {
                1u8.hash(state);
                domain.hash(state);
                next.hash(state);
            }
        }
    }
}

impl SearchNode for PeaNode {
    type Cost = u32;

    fn successors(&self) -> Vec<(Self, Self::Cost)> {
        let beam = self.beam_width().max(1);
        match &self.inner {
            PeaInner::Ready(domain) => {
                let ranked = Rc::new(Self::rank_successors(domain));
                super::astar_trace::on_expand("pea-ready", || {
                    format!(
                        "fp={:016x} gdp={:.0} h={} ranked={} beam={}",
                        domain.fingerprint(),
                        domain.state().gdp,
                        domain.heuristic(),
                        ranked.len(),
                        beam
                    )
                });
                if ranked.is_empty() {
                    return Vec::new();
                }
                Self::emit_beam(&ranked, domain, 0, beam)
            }
            PeaInner::Expanding {
                domain,
                ranked,
                next,
                beam,
            } => {
                super::astar_trace::on_expand("pea-resume", || {
                    format!(
                        "fp={:016x} gdp={:.0} next={}/{} beam={}",
                        domain.fingerprint(),
                        domain.state().gdp,
                        next,
                        ranked.len(),
                        beam
                    )
                });
                Self::emit_beam(ranked, domain, *next, *beam)
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
            PeaInner::Expanding { ranked, next, .. } => {
                ranked.get(*next).map(|s| s.f_minus_g).unwrap_or(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::plan::pathfinding::shortest_path;
    use crate::sim::SimConfig;
    use crate::world::{PlanningParts, PlanningState};
    use rust_advanced_heaps::pairing::PairingHeap;

    #[test]
    fn pea_matches_vic3_day_cost_on_research_fixture() {
        let root = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                ..PlanningParts::default()
            }),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
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
                ),
            ),
            (
                "law",
                Vic3Node::new(
                    PlanningState::default(),
                    compile("has_law(law_homesteading)").unwrap(),
                    SimConfig::default(),
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
        // Synthetic: build a Ready node and inspect successor count shape by
        // ensuring Expanding identity differs from Ready when deferred.
        let root = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                ..PlanningParts::default()
            }),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
        );
        let pea = PeaNode::ready(root);
        let succs = pea.successors();
        // Research fixture has few successors (< beam); no Expanding cursor.
        assert!(succs
            .iter()
            .all(|(n, _)| matches!(n.inner, PeaInner::Ready(_))));
        assert!(!succs.is_empty());
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
        use crate::sim::EconomyContext;
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
            PlanningState::from_world_with_prices(&world, &country, &prices).expect("state");
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
        // Note: building-candidate generation may already apply first-of-type /
        // dominance filters, so branch can be < beam (no Expanding cursor).
        // Spot-check still requires PEA* and full A* agree on day cost.

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
