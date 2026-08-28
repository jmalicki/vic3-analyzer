//! Display labels for script ids (states, buildings, professions, …).
//!
//! Prefer defs localization when present; otherwise humanize the id
//! (`STATE_BRANDENBURG` → `Brandenburg`) so SQL `region_name` (lookup),
//! `label` (owned-slice display), and alert titles share one dialect.

use std::cmp::Ordering;
use std::collections::HashMap;

use vic3_defs::GameDefs;

use crate::result::StateInfo;

/// Localized label for `id`, or [`vic3_defs::pretty_id`] when defs have no entry.
pub fn script_label(defs: &GameDefs, id: &str) -> String {
    defs.display_label(id)
}

pub use vic3_defs::pretty_id;

/// Country demonym/adjective from `{TAG}_ADJ` localization (e.g. `PRU` → Prussian).
pub(crate) fn country_adjective(defs: &GameDefs, tag: &str) -> Option<String> {
    defs.labels.get(&format!("{tag}_ADJ")).cloned()
}

/// Prefix minority holders of a shared region with the owner demonym.
///
/// Majority = largest [`StateInfo::arable_land`] (missing as 0); ties keep the
/// lowest `state.id` unprefixed. Matches Vic3's "Prussian Rhineland" pattern
/// without parsing province lists. Mutates [`StateInfo::label`].
pub(crate) fn apply_split_state_demonyms(
    states: &mut [StateInfo],
    tags_by_country: &HashMap<u32, &str>,
    defs: &GameDefs,
) {
    let mut by_region: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, state) in states.iter().enumerate() {
        if let Some(region) = state.region_name.as_deref() {
            by_region.entry(region).or_default().push(i);
        }
    }

    let mut renames: Vec<(usize, String)> = Vec::new();
    for indices in by_region.values() {
        if indices.len() < 2 {
            continue;
        }
        let Some(majority) = indices
            .iter()
            .copied()
            .max_by(|&a, &b| cmp_majority(&states[a], &states[b]))
        else {
            continue;
        };
        for &i in indices {
            if i == majority {
                continue;
            }
            let Some(country_id) = states[i].country_id else {
                continue;
            };
            let Some(tag) = tags_by_country.get(&country_id) else {
                continue;
            };
            let Some(adj) = country_adjective(defs, tag) else {
                continue;
            };
            if states[i].label.is_empty() {
                continue;
            }
            renames.push((i, format!("{adj} {}", states[i].label)));
        }
    }

    for (i, name) in renames {
        states[i].label = name;
    }
}

fn cmp_majority(a: &StateInfo, b: &StateInfo) -> Ordering {
    let aa = a.arable_land.unwrap_or(0.0);
    let ba = b.arable_land.unwrap_or(0.0);
    aa.partial_cmp(&ba)
        .unwrap_or(Ordering::Equal)
        // On equal arable land, prefer the lower state id as majority.
        .then_with(|| b.id.cmp(&a.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vic3_defs::GameDefs;

    fn defs_with(labels: &[(&str, &str)]) -> GameDefs {
        let mut defs = GameDefs::default();
        for (k, v) in labels {
            defs.labels.insert((*k).into(), (*v).into());
        }
        defs
    }

    fn state(id: u32, region: &str, name: &str, country_id: u32, arable: Option<f64>) -> StateInfo {
        StateInfo {
            id,
            region_name: Some(region.into()),
            region_label: name.into(),
            label: name.into(),
            country_id: Some(country_id),
            market_id: None,
            arable_land: arable,
            infrastructure: None,
            infrastructure_usage: None,
        }
    }

    #[test]
    fn script_label_falls_back_when_missing() {
        let defs = GameDefs::default();
        assert_eq!(script_label(&defs, "STATE_ALSACE"), "Alsace");
    }

    #[test]
    fn country_adjective_reads_adj_key() {
        let defs = defs_with(&[("PRU", "Prussia"), ("PRU_ADJ", "Prussian")]);
        assert_eq!(country_adjective(&defs, "PRU").as_deref(), Some("Prussian"));
        assert_eq!(country_adjective(&defs, "FRA"), None);
    }

    #[test]
    fn split_demonym_prefixes_minority_only() {
        let defs = defs_with(&[
            ("PRU_ADJ", "Prussian"),
            ("FRA_ADJ", "French"),
            ("STATE_RHINE", "Rhineland"),
        ]);
        let mut states = vec![
            state(10, "STATE_RHINE", "Rhineland", 1, Some(80.0)),
            state(20, "STATE_RHINE", "Rhineland", 2, Some(20.0)),
            state(30, "STATE_OTHER", "Other", 2, Some(5.0)),
        ];
        let tags: HashMap<u32, &str> = [(1, "FRA"), (2, "PRU")].into_iter().collect();
        apply_split_state_demonyms(&mut states, &tags, &defs);
        assert_eq!(states[0].label, "Rhineland");
        assert_eq!(states[1].label, "Prussian Rhineland");
        assert_eq!(states[2].label, "Other");
    }

    #[test]
    fn split_demonym_tie_keeps_lowest_id_bare() {
        let defs = defs_with(&[("PRU_ADJ", "Prussian"), ("FRA_ADJ", "French")]);
        let mut states = vec![
            state(20, "STATE_RHINE", "Rhineland", 2, Some(50.0)),
            state(10, "STATE_RHINE", "Rhineland", 1, Some(50.0)),
        ];
        let tags: HashMap<u32, &str> = [(1, "FRA"), (2, "PRU")].into_iter().collect();
        apply_split_state_demonyms(&mut states, &tags, &defs);
        assert_eq!(states[0].label, "Prussian Rhineland");
        assert_eq!(states[1].label, "Rhineland");
    }

    #[test]
    fn split_demonym_skips_when_adjective_missing() {
        let defs = GameDefs::default();
        let mut states = vec![
            state(1, "STATE_RHINE", "Rhineland", 1, Some(80.0)),
            state(2, "STATE_RHINE", "Rhineland", 2, Some(20.0)),
        ];
        let tags: HashMap<u32, &str> = [(1, "FRA"), (2, "PRU")].into_iter().collect();
        apply_split_state_demonyms(&mut states, &tags, &defs);
        assert_eq!(states[0].label, "Rhineland");
        assert_eq!(states[1].label, "Rhineland");
    }
}
