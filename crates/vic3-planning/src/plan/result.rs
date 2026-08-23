//! Serializable planning inputs, results, and local archive records.
//!
//! [`PlanOpts`] / [`PlanResult`] are the shared contract for CLI, wasm, Tauri,
//! and SQL `plan(goal [, max_days [, label]])` (label accepted, not emitted).
//! [`plan`] runs PEA*-wrapped A* via [`PeaNode`] / [`Vic3Node`]; failures are
//! [`PlanError`].

use super::{pathfinding::shortest_path, PeaNode, Vic3Node};
use crate::goals::Goal;
use crate::sim::{Action, EconomyContext, SimConfig};
use crate::world::PlanningState;
use rust_advanced_heaps::pairing::PairingHeap;
use rust_advanced_heaps::pathfinding::SearchNode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Shared planner options (CLI / wasm JSON; SQL mirrors `goal` + `max_days`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOpts {
    /// DSL source compiled by [`crate::goals::compile`].
    pub goal: String,
    /// Reject plans whose day cost exceeds this (default 3650).
    #[serde(default = "default_max_days")]
    pub max_days: u32,
    /// Archive / UI label; ignored by SQL `plan()` row shape.
    #[serde(default)]
    pub label: Option<String>,
}

const fn default_max_days() -> u32 {
    3650
}

/// One simulator action on the selected shortest path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    /// Total elapsed days after this action.
    pub day: u32,
    pub action: Action,
}

/// Stable JSON payload emitted by both CLI and wasm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanResult {
    pub day_cost: u32,
    pub actions: Vec<PlanStep>,
    pub limitations: Vec<String>,
    pub residual: f64,
}

/// One locally archived analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRecord {
    pub id: String,
    pub created_at: String,
    pub label: Option<String>,
    pub kind: String,
    pub fingerprint: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub filename: Option<String>,
    pub opts: Value,
    pub result: Value,
    pub limitations: Vec<String>,
    pub parent_id: Option<String>,
    pub blob: Option<Value>,
}

/// A stored-result comparison shared by the CLI and web UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareResult {
    pub left: String,
    pub right: String,
    pub same_fingerprint: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_cost_delta: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prices: Vec<PriceDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<GapDiff>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDiff {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<PlanStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<PlanStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceDelta {
    pub good: String,
    pub delta: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapStatus {
    StillFailing,
    Cleared,
    NewlyFailing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapDiff {
    pub atom: Value,
    pub status: GapStatus,
}

#[derive(Deserialize)]
struct StoredPricesResult {
    goods: Vec<StoredGoodPrice>,
}

#[derive(Deserialize)]
struct StoredGoodPrice {
    id: String,
    price: f64,
}

/// Compare archived result JSON without re-running the planner or price solver.
pub fn compare(left: &AnalysisRecord, right: &AnalysisRecord) -> CompareResult {
    let mut result = CompareResult {
        left: left.id.clone(),
        right: right.id.clone(),
        same_fingerprint: left.fingerprint == right.fingerprint,
        day_cost_delta: None,
        actions: Vec::new(),
        prices: Vec::new(),
        gaps: Vec::new(),
    };

    if left.kind == "plan" && right.kind == "plan" {
        if let (Ok(left_plan), Ok(right_plan)) = (
            serde_json::from_value::<PlanResult>(left.result.clone()),
            serde_json::from_value::<PlanResult>(right.result.clone()),
        ) {
            result.day_cost_delta =
                Some(i64::from(right_plan.day_cost) - i64::from(left_plan.day_cost));
            result.actions = align_actions(&left_plan.actions, &right_plan.actions);
        }
    } else if matches!(left.kind.as_str(), "prices" | "what_if")
        && matches!(right.kind.as_str(), "prices" | "what_if")
    {
        if let (Ok(left_prices), Ok(right_prices)) = (
            serde_json::from_value::<StoredPricesResult>(left.result.clone()),
            serde_json::from_value::<StoredPricesResult>(right.result.clone()),
        ) {
            for left_good in left_prices.goods {
                if let Some(right_good) = right_prices
                    .goods
                    .iter()
                    .find(|good| good.id == left_good.id)
                {
                    let delta = right_good.price - left_good.price;
                    if delta != 0.0 {
                        result.prices.push(PriceDelta {
                            good: left_good.id,
                            delta,
                        });
                    }
                }
            }
        }
    } else if left.kind == "gaps" && right.kind == "gaps" && left != right {
        let left_gaps = stored_gaps(&left.result);
        let right_gaps = stored_gaps(&right.result);
        for atom in &left_gaps {
            result.gaps.push(GapDiff {
                atom: atom.clone(),
                status: if right_gaps.contains(atom) {
                    GapStatus::StillFailing
                } else {
                    GapStatus::Cleared
                },
            });
        }
        for atom in right_gaps {
            if !left_gaps.contains(&atom) {
                result.gaps.push(GapDiff {
                    atom,
                    status: GapStatus::NewlyFailing,
                });
            }
        }
    }

    result
}

fn stored_gaps(result: &Value) -> Vec<Value> {
    result
        .get("gaps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn align_actions(left: &[PlanStep], right: &[PlanStep]) -> Vec<ActionDiff> {
    let mut lengths = vec![vec![0; right.len() + 1]; left.len() + 1];
    for left_index in (0..left.len()).rev() {
        for right_index in (0..right.len()).rev() {
            lengths[left_index][right_index] =
                if left[left_index].action == right[right_index].action {
                    lengths[left_index + 1][right_index + 1] + 1
                } else {
                    lengths[left_index + 1][right_index].max(lengths[left_index][right_index + 1])
                };
        }
    }

    let (mut left_index, mut right_index) = (0, 0);
    let mut differences = Vec::new();
    while left_index < left.len() || right_index < right.len() {
        if left_index < left.len()
            && right_index < right.len()
            && left[left_index].action == right[right_index].action
        {
            if left[left_index] != right[right_index] {
                differences.push(ActionDiff {
                    left: Some(left[left_index].clone()),
                    right: Some(right[right_index].clone()),
                });
            }
            left_index += 1;
            right_index += 1;
        } else if right_index < right.len()
            && (left_index == left.len()
                || lengths[left_index][right_index + 1] >= lengths[left_index + 1][right_index])
        {
            differences.push(ActionDiff {
                left: None,
                right: Some(right[right_index].clone()),
            });
            right_index += 1;
        } else {
            differences.push(ActionDiff {
                left: Some(left[left_index].clone()),
                right: None,
            });
            left_index += 1;
        }
    }
    differences
}

/// Planner failures reported consistently by CLI, wasm, and SQL.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// A* found no path under the current sim model.
    #[error("goal is unreachable under the current simulation model")]
    Unreachable,
    /// Shortest path exists but exceeds [`PlanOpts::max_days`].
    #[error("shortest plan costs {cost} days, exceeding --max-days {max_days}")]
    MaxDays { cost: u32, max_days: u32 },
    /// Path nodes were not adjacent under [`crate::sim::successors`] (bug).
    #[error("planner path contains an unknown state transition")]
    UnknownTransition,
}

/// Run Vic3 shortest-path search and retain simulator actions.
///
/// Cost is total event-wait days. Decision edges contribute 0.
pub fn plan(
    state: PlanningState,
    goal: Goal,
    config: SimConfig,
    max_days: u32,
    residual: f64,
    limitations: Vec<String>,
) -> Result<PlanResult, PlanError> {
    let root = Vic3Node::new(state, goal, config);
    plan_from_root(root, max_days, residual, limitations)
}

/// Run shortest-path search with immutable economy context for building actions.
pub fn plan_with_economy(
    state: PlanningState,
    goal: Goal,
    config: SimConfig,
    economy: EconomyContext,
    max_days: u32,
    residual: f64,
    limitations: Vec<String>,
) -> Result<PlanResult, PlanError> {
    let root = Vic3Node::new_with_economy(state, goal, config, economy);
    plan_from_root(root, max_days, residual, limitations)
}

fn plan_from_root(
    root: Vic3Node,
    max_days: u32,
    residual: f64,
    limitations: Vec<String>,
) -> Result<PlanResult, PlanError> {
    super::astar_trace::reset();
    if super::astar_trace::enabled() {
        let branch = root.sim_successors().len();
        eprintln!(
            "[astar] plan start gdp={:.0} h={} sim_branch={} max_days={}",
            root.state().gdp,
            root.heuristic(),
            branch,
            max_days
        );
    }
    let (pea_path, day_cost) = shortest_path::<_, PairingHeap<_, _>>(&PeaNode::ready(root))
        .ok_or(PlanError::Unreachable)?;
    if super::astar_trace::enabled() {
        eprintln!(
            "[astar] plan done expands={} day_cost={} path_len={}",
            super::astar_trace::expands(),
            day_cost,
            pea_path.len()
        );
    }
    if day_cost > max_days {
        return Err(PlanError::MaxDays {
            cost: day_cost,
            max_days,
        });
    }

    // Drop PEA* expansion cursors; keep domain fingerprint changes only.
    let mut domain_path: Vec<&Vic3Node> = Vec::with_capacity(pea_path.len());
    for node in &pea_path {
        let domain = node.domain();
        if domain_path
            .last()
            .is_some_and(|prev| prev.fingerprint() == domain.fingerprint())
        {
            continue;
        }
        domain_path.push(domain);
    }

    let mut elapsed = 0;
    let mut actions = Vec::with_capacity(domain_path.len().saturating_sub(1));
    for pair in domain_path.windows(2) {
        let from = pair[0];
        let to = pair[1];
        let edge = from
            .sim_successors()
            .into_iter()
            .find(|edge| edge.state.fingerprint() == to.fingerprint())
            .ok_or(PlanError::UnknownTransition)?;
        elapsed += u32::from(edge.days);
        actions.push(PlanStep {
            day: elapsed,
            action: edge.action,
        });
    }

    Ok(PlanResult {
        day_cost,
        actions,
        limitations,
        residual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::world::{PlanningParts, PlanningState};
    use serde_json::json;

    #[test]
    fn plan_result_contains_queue_and_wait_actions() {
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        let result = plan(
            state,
            compile("research(tech=nitroglycerin)").unwrap(),
            SimConfig::default(),
            1000,
            0.01,
            vec!["Frozen world".into()],
        )
        .unwrap();

        assert_eq!(result.day_cost, 365);
        assert_eq!(result.actions.len(), 2);
        assert!(matches!(result.actions[0].action, Action::QueueTech { .. }));
        assert!(matches!(
            result.actions[1].action,
            Action::WaitForEvent { days: 365, .. }
        ));
        assert_eq!(result.actions[1].day, 365);
    }

    fn record(id: &str, day_cost: u32) -> AnalysisRecord {
        AnalysisRecord {
            id: id.into(),
            created_at: "2026-08-15T12:00:00Z".into(),
            label: Some("rush".into()),
            kind: "plan".into(),
            fingerprint: "abc123".into(),
            date: Some("1840.2.3".into()),
            country: Some("FRA".into()),
            filename: Some("campaign.v3".into()),
            opts: json!({"goal": "research(tech=nitroglycerin)"}),
            result: json!({
                "day_cost": day_cost,
                "actions": [],
                "limitations": [],
                "residual": 0.0
            }),
            limitations: vec![],
            parent_id: None,
            blob: None,
        }
    }

    #[test]
    fn i9_analysis_record_json_round_trip_preserves_contract_fields() {
        let original = record("record-1", 365);
        let decoded: AnalysisRecord =
            serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();

        assert_eq!(decoded.id, original.id);
        assert_eq!(decoded.fingerprint, original.fingerprint);
        assert_eq!(decoded.kind, original.kind);
        assert_eq!(decoded.opts, original.opts);
        assert_eq!(decoded.result, original.result);
    }

    #[test]
    fn i9_compare_self_has_zero_cost_and_empty_diffs() {
        let original = record("record-1", 365);
        let diff = compare(&original, &original);

        assert_eq!(diff.day_cost_delta, Some(0));
        assert!(diff.actions.is_empty());
        assert!(diff.prices.is_empty());
        assert!(diff.gaps.is_empty());
    }

    #[test]
    fn plan_fixture_pair_has_known_day_cost_delta() {
        let left = record("left", 365);
        let right = record("right", 480);

        assert_eq!(compare(&left, &right).day_cost_delta, Some(115));
    }
}
