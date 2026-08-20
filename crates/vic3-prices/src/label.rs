//! Display labels for script ids (states, buildings, professions, …).
//!
//! Prefer defs localization when present; otherwise humanize the id
//! (`STATE_BRANDENBURG` → `Brandenburg`) so SQL `region_name` and alert
//! titles share one formatting dialect.

use vic3_defs::GameDefs;

/// Localized label for `id`, or [`pretty_id`] when defs have no entry.
pub(crate) fn script_label(defs: &GameDefs, id: &str) -> String {
    defs.labels
        .get(id)
        .cloned()
        .unwrap_or_else(|| pretty_id(id))
}

/// Humanize a Paradox script id for display when localization is missing.
pub(crate) fn pretty_id(id: &str) -> String {
    let trimmed = id
        .strip_prefix("STATE_")
        .or_else(|| id.strip_prefix("building_"))
        .or_else(|| id.strip_prefix("pm_"))
        .or_else(|| id.strip_prefix("popneed_"))
        .unwrap_or(id);
    trimmed
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vic3_defs::GameDefs;

    #[test]
    fn pretty_id_strips_state_prefix() {
        assert_eq!(pretty_id("STATE_BRANDENBURG"), "Brandenburg");
        assert_eq!(pretty_id("building_rye_farm"), "Rye Farm");
    }

    #[test]
    fn script_label_falls_back_when_missing() {
        let defs = GameDefs::default();
        assert_eq!(script_label(&defs, "STATE_ALSACE"), "Alsace");
    }
}
