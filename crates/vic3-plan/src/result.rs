//! Serializable planning inputs, results, and local archive records.

use crate::{pathfinding::shortest_path, Vic3Node};
use rust_advanced_heaps::pairing::PairingHeap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vic3_goals::Goal;
use vic3_sim::{Action, SimConfig};
use vic3_world::PlanningState;

/// Shared planner options used by CLI and wasm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOpts {
    pub goal: String,
    #[serde(default = "default_max_days")]
    pub max_days: u32,
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

/// Planner failures that can be reported consistently by clients.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("goal is unreachable under the current simulation model")]
    Unreachable,
    #[error("shortest plan costs {cost} days, exceeding --max-days {max_days}")]
    MaxDays { cost: u32, max_days: u32 },
    #[error("planner path contains an unknown state transition")]
    UnknownTransition,
}

/// Run the Vic3 shortest-path search and retain the simulator actions.
pub fn plan(
    state: PlanningState,
    goal: Goal,
    config: SimConfig,
    max_days: u32,
    residual: f64,
    limitations: Vec<String>,
) -> Result<PlanResult, PlanError> {
    let root = Vic3Node::new(state, goal, config);
    let (path, day_cost) =
        shortest_path::<_, PairingHeap<_, _>>(&root).ok_or(PlanError::Unreachable)?;
    if day_cost > max_days {
        return Err(PlanError::MaxDays {
            cost: day_cost,
            max_days,
        });
    }

    let mut elapsed = 0;
    let mut actions = Vec::with_capacity(path.len().saturating_sub(1));
    for pair in path.windows(2) {
        let from = &pair[0];
        let to = &pair[1];
        let edge = vic3_sim::successors(from.state(), from.goal(), from.config())
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
    use vic3_goals::compile;
    use vic3_world::{PlanningParts, PlanningState};

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
}
