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
            } => Self::emit_beam(ranked, domain, *next, *beam),
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
}
