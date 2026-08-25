//! Technology prerequisite expansion against [`vic3_defs::GameDefs`].
//!
//! Goal DSL keeps a leaf `HasTech(id)` string; planning expands missing
//! ancestors into gaps / eligible [`crate::sim::Action::QueueTech`] targets
//! so A* queues prereqs before the leaf (GOAP-style).

use std::collections::{BTreeSet, VecDeque};

use vic3_defs::GameDefs;

use crate::goals::SimpleSubgoal;
use crate::world::PlanningState;

/// Missing techs that must be researched to own `leaf`, including `leaf`.
///
/// Walks `unlocking_technologies` / [`vic3_defs::Technology::prerequisites`]
/// transitively. Techs already in [`PlanningState::techs`] are omitted.
/// Unknown ids (absent from defs) are treated as leaves with no parents so
/// planning still queues them when the goal names them.
///
/// Order is deterministic: ancestors before dependents when the graph is a
/// DAG; cycles yield each id at most once.
pub fn missing_tech_closure(leaf: &str, state: &PlanningState, defs: &GameDefs) -> Vec<String> {
    if leaf.is_empty() || state.has_tech(leaf) {
        return Vec::new();
    }
    let mut missing = BTreeSet::new();
    let mut stack = vec![leaf.to_string()];
    let mut visiting = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if state.has_tech(&id) || !missing.insert(id.clone()) {
            continue;
        }
        if !visiting.insert(id.clone()) {
            continue;
        }
        if let Some(tech) = defs.technologies.get(&id) {
            for prereq in &tech.prerequisites {
                if !state.has_tech(prereq) {
                    stack.push(prereq.clone());
                }
            }
        }
    }
    topo_sort_missing(&missing, defs)
}

/// Whether every prerequisite of `tech` is already owned (or `tech` has none).
///
/// Unknown ids (absent from defs) return `true` so leaf-only plans still queue
/// when the goal names a tech without a graph entry.
pub fn tech_prereqs_satisfied(tech: &str, state: &PlanningState, defs: &GameDefs) -> bool {
    let Some(def) = defs.technologies.get(tech) else {
        return true;
    };
    def.prerequisites.iter().all(|p| state.has_tech(p))
}

/// Expand open `HasTech` atoms into the full missing ancestor set.
///
/// Non-tech simple subgoals are preserved. Duplicate tech ids collapse. Without a
/// useful graph (empty technologies map), returns `atoms` unchanged aside
/// from dropping already-owned techs.
pub fn expand_tech_gap_simple_subgoals(
    atoms: &[SimpleSubgoal],
    state: &PlanningState,
    defs: &GameDefs,
) -> Vec<SimpleSubgoal> {
    if defs.technologies.is_empty() {
        return atoms
            .iter()
            .filter(|atom| match atom {
                SimpleSubgoal::HasTech(tech) => !state.has_tech(tech),
                _ => true,
            })
            .cloned()
            .collect();
    }
    let mut out = Vec::new();
    let mut seen_techs = BTreeSet::new();
    for atom in atoms {
        match atom {
            SimpleSubgoal::HasTech(tech) => {
                for id in missing_tech_closure(tech, state, defs) {
                    if seen_techs.insert(id.clone()) {
                        out.push(SimpleSubgoal::HasTech(id));
                    }
                }
            }
            other => out.push(other.clone()),
        }
    }
    out
}

/// Innovation cost for `tech` when defs provide a finite non-negative cost.
pub fn tech_research_cost(tech: &str, defs: &GameDefs) -> Option<f64> {
    defs.technologies
        .get(tech)?
        .cost
        .filter(|c| c.is_finite() && *c >= 0.0)
}

fn topo_sort_missing(missing: &BTreeSet<String>, defs: &GameDefs) -> Vec<String> {
    let mut indegree: std::collections::BTreeMap<String, usize> =
        missing.iter().map(|id| (id.clone(), 0)).collect();
    let mut children: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for id in missing {
        if let Some(tech) = defs.technologies.get(id) {
            for prereq in &tech.prerequisites {
                if missing.contains(prereq) {
                    *indegree.get_mut(id).expect("id in missing") += 1;
                    children.entry(prereq.clone()).or_default().push(id.clone());
                }
            }
        }
    }
    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    // Stable among zero-indegree ids.
    let mut ready: Vec<String> = queue.drain(..).collect();
    ready.sort();
    queue.extend(ready);

    let mut ordered = Vec::with_capacity(missing.len());
    while let Some(id) = queue.pop_front() {
        ordered.push(id.clone());
        if let Some(deps) = children.get(&id) {
            let mut unlocked = Vec::new();
            for child in deps {
                if let Some(d) = indegree.get_mut(child) {
                    *d = d.saturating_sub(1);
                    if *d == 0 {
                        unlocked.push(child.clone());
                    }
                }
            }
            unlocked.sort();
            queue.extend(unlocked);
        }
    }
    // Cycle / leftover: append remaining ids sorted.
    if ordered.len() < missing.len() {
        for id in missing {
            if !ordered.contains(id) {
                ordered.push(id.clone());
            }
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{PlanningParts, PlanningState};
    use std::collections::BTreeMap;
    use vic3_defs::Technology;

    fn fixture_defs() -> GameDefs {
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
    fn missing_closure_includes_ancestors() {
        let defs = fixture_defs();
        let state = PlanningState::default();
        assert_eq!(
            missing_tech_closure("nitroglycerin", &state, &defs),
            ["manufacturies", "shaft_mining", "nitroglycerin"]
        );
    }

    #[test]
    fn owned_ancestors_are_omitted() {
        let defs = fixture_defs();
        let state = PlanningState::from_parts(PlanningParts {
            techs: vec!["manufacturies".into()],
            ..PlanningParts::default()
        });
        assert_eq!(
            missing_tech_closure("nitroglycerin", &state, &defs),
            ["shaft_mining", "nitroglycerin"]
        );
    }

    #[test]
    fn prereqs_gate_eligibility() {
        let defs = fixture_defs();
        let empty = PlanningState::default();
        assert!(tech_prereqs_satisfied("manufacturies", &empty, &defs));
        assert!(!tech_prereqs_satisfied("shaft_mining", &empty, &defs));
        let with_root = PlanningState::from_parts(PlanningParts {
            techs: vec!["manufacturies".into()],
            ..PlanningParts::default()
        });
        assert!(tech_prereqs_satisfied("shaft_mining", &with_root, &defs));
    }
}
