//! Rank sim successors for search candidate bags (cheap + emit keys).
//!
//! Sits between [`super::progress_h`] (pure residual / bag-score math) and
//! search adapters such as [`super::pea`] (beam width, deferred expansion
//! cursors). Adapters call here to build a ranked bag and to rescore on emit;
//! they should not embed [`progress_h`] scoring rules directly.
//!
//! Design: [`planning-progress-heuristic.md`](../../../docs/planning-progress-heuristic.md).

use super::progress_h::{self, CheapBagCurr};
use super::Vic3Node;
use crate::sim::Action;
use rust_advanced_heaps::pathfinding::SearchNode;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// One ranked row in a country-wide candidate bag (cheap key at expand time).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RankedBagEntry {
    pub action: Action,
    pub days: u16,
    /// Cheap ranking key: `edge + follow-on guesstimate` (lower is better).
    pub cheap_rank_key: u32,
    /// Deterministic tie-break when `cheap_rank_key` matches.
    pub tie: u64,
}

/// Deterministic tie-break for bag rows that share the same rank key.
pub fn tie_break(action: &Action, days: u16) -> u64 {
    let mut hasher = DefaultHasher::new();
    action.hash(&mut hasher);
    days.hash(&mut hasher);
    hasher.finish()
}

/// Build a cheap-ranked bag from all sim successors of `domain`.
///
/// On [`progress_h::ProgressGapError`], falls back to admissible `edge + h_adm`
/// per successor so search still expands.
pub fn cheap_rank_bag(domain: &Vic3Node) -> Vec<RankedBagEntry> {
    let successors = domain.sim_successors();
    let state_curr = domain.state();
    let curr = match CheapBagCurr::new(
        domain.goal(),
        state_curr,
        domain.config(),
        domain.economy(),
        domain.gdp_for_rates(),
    ) {
        Ok(curr) => curr,
        Err(err) => {
            tracing::warn!(
                target: "vic3_planning::bag_rank",
                ?err,
                "CheapBagCurr failed; bag falls back to h_adm per edge"
            );
            return successors
                .into_iter()
                .map(|successor| {
                    let node = Vic3Node::with_shared_context(successor.state, domain);
                    let edge = u32::from(successor.days);
                    let tie = tie_break(&successor.action, successor.days);
                    RankedBagEntry {
                        action: successor.action,
                        days: successor.days,
                        cheap_rank_key: edge.saturating_add(node.heuristic()),
                        tie,
                    }
                })
                .collect();
        }
    };

    successors
        .into_iter()
        .map(|successor| RankedBagEntry {
            cheap_rank_key: progress_h::cheap_bag_score(&successor.action, successor.days, &curr),
            tie: tie_break(&successor.action, successor.days),
            action: successor.action,
            days: successor.days,
        })
        .collect()
}

/// Apply one bag row and set anticipated complete GDP on the child for ranking.
///
/// Returns `(child, edge_days)`. Child state stays post-enqueue; [`Vic3Node::gdp_for_rates`]
/// anticipates speculative complete when available.
pub fn emit_child(domain: &Vic3Node, entry: &RankedBagEntry) -> Option<(Vic3Node, u32)> {
    let state = domain.apply_action(&entry.action)?;
    let mut child = Vic3Node::with_shared_context(state, domain);

    let gdp_curr = domain.gdp_for_rates();
    match crate::sim::speculative_completed_state(
        domain.state(),
        &entry.action,
        domain.economy(),
        domain.config(),
    ) {
        Ok(completed) => {
            let emit_gdp_delta = completed.gdp - gdp_curr;
            child = child.with_gdp_for_rates(gdp_curr + emit_gdp_delta);
        }
        Err(err) => {
            tracing::debug!(
                target: "vic3_planning::bag_rank",
                ?err,
                action = ?entry.action,
                "speculative_completed_state failed; emit without GDP anticipation"
            );
        }
    }

    Some((child, u32::from(entry.days)))
}

/// Emit-time ranking key: full residual on speculative complete, with fallback.
pub fn emit_rank_key(domain: &Vic3Node, entry: &RankedBagEntry, child: &Vic3Node) -> u32 {
    let completed = crate::sim::speculative_completed_state(
        domain.state(),
        &entry.action,
        domain.economy(),
        domain.config(),
    )
    .ok();

    if let Some(completed) = completed {
        if let Ok(key) = progress_h::emit_bag_score(
            entry.days,
            domain.goal(),
            &completed,
            domain.config(),
            domain.economy(),
        ) {
            return key;
        }
    }

    let residual = progress_h::rank_heuristic_with_gdp_for_rates(
        domain.goal(),
        child.state(),
        domain.config(),
        domain.economy(),
        child.gdp_for_rates(),
    )
    .unwrap_or_else(|_| child.heuristic());
    u32::from(entry.days).saturating_add(residual)
}

/// True when emit ranking is worse than the best deferred cheap bag score.
///
/// Design: emit key > deferred_min_cheap → warn (cheap under-ranked a rival).
/// Emit/rebuild follow-on better than cheap is expected — do not warn.
pub fn emit_rank_exceeds_deferred_cheap(emit_rank_key: u32, deferred_min_cheap: u32) -> bool {
    emit_rank_key > deferred_min_cheap
}

/// Record for tracing when emit rescore beats the best deferred cheap key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitDeferredCheapMismatch {
    pub cheap_rank_key: u32,
    pub emit_rank_key: u32,
    pub deferred_min_cheap: u32,
}

/// Build a mismatch record when a warn should fire; `None` if no warn.
pub fn emit_deferred_cheap_mismatch(
    cheap_rank_key: u32,
    emit_rank_key: u32,
    deferred_min_cheap: Option<u32>,
) -> Option<EmitDeferredCheapMismatch> {
    let deferred_min = deferred_min_cheap?;
    if !emit_rank_exceeds_deferred_cheap(emit_rank_key, deferred_min) {
        return None;
    }
    Some(EmitDeferredCheapMismatch {
        cheap_rank_key,
        emit_rank_key,
        deferred_min_cheap: deferred_min,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::sim::{EconomyContext, SimConfig};
    use crate::world::{PlanningParts, PlanningState};

    #[test]
    fn emit_rank_exceeds_deferred_cheap_only_when_worse() {
        assert!(!emit_rank_exceeds_deferred_cheap(10, 10));
        assert!(!emit_rank_exceeds_deferred_cheap(9, 10));
        assert!(emit_rank_exceeds_deferred_cheap(11, 10));
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
                cheap_rank_key: 5,
                emit_rank_key: 20,
                deferred_min_cheap: 10,
            })
        );
        assert_eq!(emit_deferred_cheap_mismatch(5, 10, Some(10)), None);
        assert_eq!(emit_deferred_cheap_mismatch(5, 9, Some(10)), None);
    }

    #[test]
    fn cheap_rank_bag_nonempty_on_research_fixture() {
        let root = Vic3Node::new(
            PlanningState::from_parts(PlanningParts {
                country: "GER".into(),
                ..PlanningParts::default()
            }),
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
            EconomyContext::empty(),
        );
        let bag = cheap_rank_bag(&root);
        assert!(!bag.is_empty());
        assert!(bag.iter().all(|e| e.cheap_rank_key > 0 || e.days == 0));
    }
}
