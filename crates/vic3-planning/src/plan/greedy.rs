//! Greedy incumbent → upper bound $U$ (calendar days).
//!
//! # Construction Sector policy
//!
//! Greedy **never** queues a **new** Construction Sector
//! (`building_construction_sector`). That hard-skip is intentional: shortage
//! greedy would not pick CS anyway (it does not produce the scarce good), and
//! inventing CS picks would scramble incremental rebuild assumptions later.
//!
//! When rebuilding greedy from a search state that **already has CS enqueued**
//! in `state.constructions`, this loop does **not** invent CS decisions. It
//! only picks non-CS 0-day decisions; otherwise it takes the shortest
//! [`Action::WaitForEvent`]. That wait path is the real construction timeline:
//! slots and construction points apply as usual, and when
//! `BuildingCompleted` fires for the in-flight CS, apply updates capacity.
//! Residual shortage builds then run under the post-CS rate.
//!
//! This differs from the **PEA cheap Construction Sector bag score**, which
//! only scales follow-on days on state_t by a construction-unit ratio and does not
//! replay the actual queue. See `docs/planning-progress-heuristic.md`.
//!
//! Ordinary [`Action::QueueBuildingLevel`] (logging camps, barracks, …) is
//! still allowed. Build picks: highest domestic shortage good by market price
//! vs defs base among allowed buildings.

use crate::construction::BUILDING_CONSTRUCTION_SECTOR;
use crate::goals::{evaluate, Goal, SimpleSubgoal};
use crate::plan::progress_h::simple_subgoal_delta;
use crate::plan::Vic3Node;
use crate::sim::{Action, EconomyContext, Successor};
use crate::world::PlanningState;

/// Run greedy from `root` until the goal or `max_days`.
///
/// Returns calendar days to first goal hit, or `None` if stuck / over budget.
///
/// Loop shape:
/// 1. If the goal already holds → done.
/// 2. Try a 0-day greedy decision (never a new Construction Sector).
/// 3. Else advance via the shortest [`Action::WaitForEvent`] (research, law,
///    hire, **or in-flight construction including an already-queued CS**).
///
/// So a current state that already enqueued Construction Sector is honored by waiting
/// out that job’s real slots / points, not by special-casing a CS “pick”.
pub fn greedy_upper_bound(root: &Vic3Node, max_days: u32) -> Option<u32> {
    let goal = root.goal().clone();
    let mut state = root.state().clone();
    let mut days = 0u32;
    let economy = root.economy();

    while days <= max_days {
        if evaluate(&goal, &state) {
            return Some(days);
        }
        let edges = crate::sim::successors(&state, &goal, root.config(), economy);

        // Prefer a helpful 0-day decision. Construction Sector is never chosen
        // here (`is_greedy_decision` / `best_shortage_build` hard-skip).
        if let Some(pick) = best_greedy_decision(&goal, &state, &edges, root.config(), economy) {
            state = pick.state;
            continue;
        }

        // No decision: take the soonest wait. When CS (or any building) is
        // already in `constructions`, sim emits a BuildingCompleted wait whose
        // apply path updates levels and construction points/day — so rebuilds
        // from an in-flight-CS state_t advance on the real timeline.
        let wait = edges
            .into_iter()
            .filter(|e| matches!(e.action, Action::WaitForEvent { .. }))
            .min_by_key(|e| e.days)?;
        days = days.saturating_add(u32::from(wait.days));
        if days > max_days {
            return None;
        }
        state = wait.state;
    }
    None
}

fn best_greedy_decision(
    goal: &Goal,
    before: &PlanningState,
    edges: &[Successor],
    config: crate::sim::SimConfig,
    economy: &EconomyContext,
) -> Option<Successor> {
    let open: Vec<_> = goal
        .simple_subgoals()
        .into_iter()
        .filter(|a| !a.eval(before))
        .collect();
    if open.is_empty() {
        return None;
    }

    // Instant / binary / PM / tax first when they help the open goal.
    if let Some(pick) = best_non_build_decision(goal, before, &open, edges, config) {
        return Some(pick);
    }

    // Builds: highest shortage good via market price / defs base among allowed.
    // Never Construction Sector — even if sim offers a CS enqueue edge.
    best_shortage_build(before, edges, economy)
}

fn best_non_build_decision(
    goal: &Goal,
    before: &PlanningState,
    open: &[&SimpleSubgoal],
    edges: &[Successor],
    config: crate::sim::SimConfig,
) -> Option<Successor> {
    let mut best: Option<(f64, Successor)> = None;
    for edge in edges {
        if matches!(edge.action, Action::QueueBuildingLevel { .. }) {
            continue;
        }
        if !is_greedy_decision(&edge.action) || edge.days != 0 {
            continue;
        }

        let mut delta = 0.0_f64;
        let mut eta = 0u32;
        for subgoal in open {
            let d = simple_subgoal_delta(subgoal, before, &edge.state).unwrap_or(0.0);
            if d > 0.0 {
                delta += d;
                continue;
            }
            if let Some(track_eta) = binary_enqueue_credit(subgoal, &edge.action, before, config) {
                delta += 1.0;
                eta = eta.max(track_eta);
            }
        }
        if delta <= 0.0 {
            if evaluate(goal, &edge.state) {
                delta = 1.0;
            } else {
                continue;
            }
        }

        let rate = if eta == 0 {
            delta * 1.0e6
        } else {
            delta / f64::from(eta.max(1))
        };

        let better = match &best {
            None => true,
            Some((best_r, _)) => rate > *best_r + f64::EPSILON,
        };
        if better {
            best = Some((rate, edge.clone()));
        }
    }
    best.map(|(_, e)| e)
}

/// Shortage-greedy build pick among 0-day `QueueBuildingLevel` edges.
///
/// Hard-skips Construction Sector so greedy never enqueues a new CS level.
/// In-flight CS already on the queue is handled by the main loop’s wait edge,
/// not by inventing a CS decision here.
fn best_shortage_build(
    before: &PlanningState,
    edges: &[Successor],
    economy: &EconomyContext,
) -> Option<Successor> {
    let mut best: Option<(f64, Successor)> = None;
    for edge in edges {
        let Action::QueueBuildingLevel { building, .. } = &edge.action else {
            continue;
        };
        if building == BUILDING_CONSTRUCTION_SECTOR || edge.days != 0 {
            continue;
        }
        let score = economy.max_output_price_over_base(before, building);
        if score <= f64::EPSILON {
            continue;
        }
        let better = match &best {
            None => true,
            Some((best_s, _)) => score > *best_s + f64::EPSILON,
        };
        if better {
            best = Some((score, edge.clone()));
        }
    }
    best.map(|(_, e)| e)
}

/// Whether greedy may take this 0-day decision.
///
/// `QueueBuildingLevel` is allowed only when the building is **not**
/// Construction Sector. Waits are never decisions (main loop handles them).
fn is_greedy_decision(action: &Action) -> bool {
    match action {
        Action::QueueBuildingLevel { building, .. } => building != BUILDING_CONSTRUCTION_SECTOR,
        Action::SwitchPm { .. }
        | Action::QueueTech { .. }
        | Action::QueueLaw { .. }
        | Action::QueueInterest { .. }
        | Action::QueueHireMilitary { .. }
        | Action::AdjustTax { .. } => true,
        _ => false,
    }
}

fn binary_enqueue_credit(
    subgoal: &SimpleSubgoal,
    action: &Action,
    before: &PlanningState,
    config: crate::sim::SimConfig,
) -> Option<u32> {
    match (subgoal, action) {
        (SimpleSubgoal::HasTech(tech), Action::QueueTech { tech: queued }) if tech == queued => {
            if before.has_tech(tech) || before.research_busy() {
                None
            } else {
                Some(u32::from(config.research_days.max(1)))
            }
        }
        (SimpleSubgoal::HasLaw(law), Action::QueueLaw { law: queued })
            if crate::world::law_key(law) == crate::world::law_key(queued) =>
        {
            if before.has_law(law) || before.law_busy() {
                None
            } else {
                Some(u32::from(config.law_days.max(1)))
            }
        }
        (SimpleSubgoal::InterestIn { kind, id }, Action::QueueInterest { kind: ak, id: aid })
            if kind == ak && id == aid =>
        {
            if before.interest_busy() {
                None
            } else {
                Some(u32::from(config.interest_days.max(1)))
            }
        }
        (SimpleSubgoal::ArmyPower { .. }, Action::QueueHireMilitary { .. })
        | (SimpleSubgoal::NavyPower { .. }, Action::QueueHireMilitary { .. }) => {
            if before.hire_busy() {
                None
            } else {
                Some(u32::from(config.army_training_days.max(1)))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::compile;
    use crate::plan::Vic3Node;
    use crate::sim::{EconomyContext, Event, SimConfig};
    use crate::world::{ConstructionQueueKind, PlanningConstruction, PlanningParts, PlanningState};

    #[test]
    fn greedy_tech_goal_finishes() {
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let root = Vic3Node::new(state, goal, SimConfig::default(), EconomyContext::empty());
        let u = greedy_upper_bound(&root, 3650).expect("greedy tech");
        assert!(u > 0);
        assert_eq!(u, u32::from(SimConfig::default().research_days.max(1)));
    }

    #[test]
    fn greedy_already_satisfied_is_zero() {
        let state = PlanningState::from_parts(PlanningParts {
            techs: ["nitroglycerin".into()].into_iter().collect(),
            ..PlanningParts::default()
        });
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let root = Vic3Node::new(state, goal, SimConfig::default(), EconomyContext::empty());
        assert_eq!(greedy_upper_bound(&root, 3650), Some(0));
    }

    #[test]
    fn greedy_skips_construction_sector_only() {
        assert!(is_greedy_decision(&Action::QueueBuildingLevel {
            building: "building_logging_camp".into(),
            state_id: 1,
        }));
        assert!(!is_greedy_decision(&Action::QueueBuildingLevel {
            building: BUILDING_CONSTRUCTION_SECTOR.into(),
            state_id: 1,
        }));
    }

    /// Decision layer: when sim offers both a *new* CS enqueue and a wait for
    /// an already-queued CS, greedy must pick neither as a decision (so the
    /// main loop takes the wait). Forged edges — no economy needed for this
    /// decision-layer assert.
    #[test]
    fn greedy_prefers_wait_over_new_cs_when_cs_already_queued() {
        let before = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            queued_building: Some(BUILDING_CONSTRUCTION_SECTOR.into()),
            constructions: vec![PlanningConstruction {
                order_id: 1,
                queue: ConstructionQueueKind::Government,
                state_id: Some(1),
                building: BUILDING_CONSTRUCTION_SECTOR.into(),
                remaining: Some(50.0),
            }],
            construction_points_per_day: 5.0,
            ..PlanningParts::default()
        });
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let after_wait = before.clone();
        let edges = vec![
            Successor {
                action: Action::QueueBuildingLevel {
                    building: BUILDING_CONSTRUCTION_SECTOR.into(),
                    state_id: 1,
                },
                days: 0,
                state: before.clone(),
            },
            Successor {
                action: Action::WaitForEvent {
                    event: Event::BuildingCompleted {
                        building: BUILDING_CONSTRUCTION_SECTOR.into(),
                        state_id: Some(1),
                    },
                    days: 10,
                },
                days: 10,
                state: after_wait,
            },
        ];

        assert!(
            best_greedy_decision(
                &goal,
                &before,
                &edges,
                SimConfig::default(),
                &EconomyContext::empty()
            )
            .is_none(),
            "greedy must not enqueue another Construction Sector; wait is for the main loop"
        );
        assert!(!is_greedy_decision(&edges[0].action));
    }

    /// Economy path: in-flight CS yields a real `BuildingCompleted` wait; greedy
    /// never picks a new CS enqueue; applying the CS wait raises throughput.
    #[test]
    fn greedy_rebuild_waits_out_in_flight_cs_with_economy() {
        use crate::test_support::{ger_state, logging_and_cs_economy};

        let mini = logging_and_cs_economy();
        let state = ger_state()
            .cs_in_flight(50.0)
            .gdp(1000.0)
            .points(1.0)
            .wood_price(30.0)
            .get();
        let goal = compile("gdp >= 1e12").unwrap();
        let points_before = state.construction_points_per_day;

        let edges = crate::sim::successors(&state, &goal, mini.config, &mini.economy);
        assert!(
            edges.iter().any(|e| matches!(
                &e.action,
                Action::WaitForEvent {
                    event: Event::BuildingCompleted { building, state_id: Some(1) },
                    ..
                } if building == BUILDING_CONSTRUCTION_SECTOR
            )),
            "economy must emit a CS BuildingCompleted wait; got {edges:?}"
        );

        if let Some(pick) = best_greedy_decision(&goal, &state, &edges, mini.config, &mini.economy)
        {
            assert!(
                !matches!(
                    &pick.action,
                    Action::QueueBuildingLevel { building, .. }
                        if building == BUILDING_CONSTRUCTION_SECTOR
                ),
                "greedy must not pick a new Construction Sector enqueue"
            );
        }

        let cs_wait = edges
            .iter()
            .find(|e| {
                matches!(
                    &e.action,
                    Action::WaitForEvent {
                        event: Event::BuildingCompleted { building, state_id: Some(1) },
                        ..
                    } if building == BUILDING_CONSTRUCTION_SECTOR
                )
            })
            .expect("CS BuildingCompleted wait");
        assert!(
            cs_wait.state.construction_points_per_day > points_before,
            "completing in-flight CS should raise points/day: before={points_before} after={}",
            cs_wait.state.construction_points_per_day
        );
    }
}
