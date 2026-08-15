//! Goal-relevant simulator successors over [`PlanningState`].
//!
//! Decision edges cost zero days. An expansion contains at most one event-wait
//! edge, selected from events already in flight. This crate deliberately does
//! not perform graph search.

use serde::{Deserialize, Serialize};
use vic3_goals::{gaps, Atom, Goal};
use vic3_world::PlanningState;

/// Tunable durations used by the compact simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimConfig {
    /// Fixed research duration in the phase-8 model.
    pub research_days: u16,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { research_days: 365 }
    }
}

/// An event which can advance the simulation clock.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Event {
    /// The technology currently in the queue completes.
    TechCompleted { tech: String },
}

/// A deterministic state transition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// Put a goal-relevant technology in the empty research queue.
    QueueTech { tech: String },
    /// Advance directly to an event already in flight.
    WaitForEvent { event: Event, days: u16 },
}

/// One edge emitted by [`successors`] or [`successors_for_atoms`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Successor {
    pub action: Action,
    /// Edge cost in days. Decision edges have cost zero.
    pub days: u16,
    pub state: PlanningState,
}

/// Generate successors relevant to the currently unsatisfied atoms of `goal`.
pub fn successors(state: &PlanningState, goal: &Goal, config: SimConfig) -> Vec<Successor> {
    let open_atoms = gaps(goal, state);
    successors_for_atoms(state, &open_atoms, config)
}

/// Generate successors from an already-computed list of open goal atoms.
///
/// Technology decisions are emitted in atom order with duplicates removed.
/// At most one wait edge is appended. A queued technology is the only in-flight
/// event represented by the compact phase-8 state; therefore an idle state has
/// no wait edge. Payday is intentionally not invented when no solvency model is
/// present.
pub fn successors_for_atoms(
    state: &PlanningState,
    open_atoms: &[Atom],
    config: SimConfig,
) -> Vec<Successor> {
    let mut result = Vec::new();
    let mut seen_techs = std::collections::BTreeSet::new();

    if state.queued_tech.is_none() {
        for atom in open_atoms {
            let Atom::HasTech(tech) = atom else {
                continue;
            };
            if state.has_tech(tech) || !seen_techs.insert(tech.clone()) {
                continue;
            }
            let action = Action::QueueTech { tech: tech.clone() };
            if let Some(next) = apply_action(state, &action) {
                result.push(Successor {
                    action,
                    days: 0,
                    state: next,
                });
            }
        }
    }

    if let Some(tech) = state.queued_tech.as_ref().filter(|queued| {
        open_atoms
            .iter()
            .any(|atom| atom.is_has_tech(queued.as_str()))
    }) {
        let days = config.research_days.max(1);
        let action = Action::WaitForEvent {
            event: Event::TechCompleted { tech: tech.clone() },
            days,
        };
        if let Some(next) = apply_action(state, &action) {
            result.push(Successor {
                action,
                days,
                state: next,
            });
        }
    }

    result
}

/// Apply an action if its preconditions hold.
///
/// The action carries all timing information, so applying it to identical
/// states always produces identical states (I8).
pub fn apply_action(state: &PlanningState, action: &Action) -> Option<PlanningState> {
    let mut next = state.clone();
    match action {
        Action::QueueTech { tech } => {
            if tech.is_empty() || next.queued_tech.is_some() || next.has_tech(tech) {
                return None;
            }
            next.queued_tech = Some(tech.clone());
        }
        Action::WaitForEvent {
            event: Event::TechCompleted { tech },
            days,
        } => {
            if *days == 0 || next.queued_tech.as_deref() != Some(tech.as_str()) {
                return None;
            }
            next.date = next.date.add_days(i32::from(*days));
            next.queued_tech = None;
            next.techs.insert(tech.clone());
        }
    }
    Some(next)
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use vic3_goals::{compile, evaluate};
    use vic3_world::{PlanningParts, Vic3Date};

    #[test]
    fn version_is_semver() {
        assert!(!super::version().is_empty());
    }

    fn state_at(day_offset: i32) -> PlanningState {
        PlanningState::from_parts(PlanningParts {
            date: Vic3Date::from_ymdh(1836, 1, 1, 0).add_days(day_offset),
            country: "GER".into(),
            ..PlanningParts::default()
        })
    }

    #[test]
    fn queue_tech_then_wait_reaches_has_tech() {
        let goal = compile("research(tech=nitroglycerin)").unwrap();
        let start = state_at(0);

        let decisions = successors(&start, &goal, SimConfig { research_days: 100 });
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].days, 0);
        assert!(matches!(
            decisions[0].action,
            Action::QueueTech { ref tech } if tech == "nitroglycerin"
        ));

        let waits = successors(&decisions[0].state, &goal, SimConfig { research_days: 100 });
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].days, 100);
        assert_eq!(start.date.days_until(&waits[0].state.date), 100);
        assert!(waits[0].state.queued_tech.is_none());
        assert!(evaluate(&goal, &waits[0].state));
    }

    #[test]
    fn i8_actions_deterministic_and_hash_stable() {
        let state = state_at(7);
        let action = Action::QueueTech {
            tech: "railways".into(),
        };
        let a = apply_action(&state, &action).unwrap();
        let b = apply_action(&state.clone(), &action.clone()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());

        let wait = Action::WaitForEvent {
            event: Event::TechCompleted {
                tech: "railways".into(),
            },
            days: 45,
        };
        let a = apply_action(&a, &wait).unwrap();
        let b = apply_action(&b, &wait).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// I6: successor dates are monotone, and idle non-solvency goals have
        /// no event-wait edge.
        #[test]
        fn i6_event_wait_date_monotone_and_not_spurious(
            day_offset in 0i32..20_000,
            research_days in any::<u16>(),
            queue_tech in any::<bool>(),
        ) {
            let goal = compile("research(tech=railways)").unwrap();
            let mut state = state_at(day_offset);
            if queue_tech {
                state.queued_tech = Some("railways".into());
            }
            let edges = successors(&state, &goal, SimConfig { research_days });

            let wait_count = edges
                .iter()
                .filter(|edge| matches!(&edge.action, Action::WaitForEvent { .. }))
                .count();
            prop_assert!(wait_count <= 1);
            for edge in &edges {
                prop_assert!(edge.state.date >= state.date);
                if matches!(&edge.action, Action::WaitForEvent { .. }) {
                    prop_assert!(edge.state.date > state.date);
                }
            }

            if !queue_tech {
                prop_assert_eq!(wait_count, 0);
            }

            let idle_atoms = [Atom::ArmyPower {
                rel: vic3_goals::Rel::Ge,
                value: 100.0,
            }];
            let idle_edges = successors_for_atoms(
                &state_at(day_offset),
                &idle_atoms,
                SimConfig { research_days },
            );
            let has_wait = idle_edges
                .iter()
                .any(|edge| matches!(&edge.action, Action::WaitForEvent { .. }));
            prop_assert!(!has_wait);
        }
    }
}
