//! Resolve human-readable labels from Paradox script ids.
//!
//! Game data and saves refer to entities by **script id** (`grain`, `STATE_RHINE`,
//! `building_rye_farm`). English localization lives in [`GameDefs::labels`], keyed
//! by that same id. When localization is missing (partial defs, modded ids), we
//! fall back to [`pretty_id`] — strip common prefixes and title-case tokens.
//!
//! Call [`GameDefs::display_label`] once when building export rows; downstream
//! SQL, alerts, and API code read the resolved `label` string directly.

use crate::GameDefs;

impl GameDefs {
    /// Display string for a script `id`.
    ///
    /// Uses [`Self::labels`] when present; otherwise [`pretty_id`]. Never returns
    /// an empty string for a non-empty id.
    pub fn display_label(&self, id: &str) -> String {
        self.labels
            .get(id)
            .cloned()
            .unwrap_or_else(|| pretty_id(id))
    }
}

/// Humanize a Paradox script id when localization is absent.
///
/// Strips known prefixes (`STATE_`, `building_`, `pm_`, `popneed_`) then splits
/// on `_` and title-cases each token (`STATE_BRANDENBURG` → `Brandenburg`).
pub fn pretty_id(id: &str) -> String {
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

    #[test]
    fn pretty_id_strips_state_prefix() {
        assert_eq!(pretty_id("STATE_BRANDENBURG"), "Brandenburg");
        assert_eq!(pretty_id("building_rye_farm"), "Rye Farm");
    }

    #[test]
    fn display_label_falls_back_when_missing() {
        let defs = GameDefs::default();
        assert_eq!(defs.display_label("STATE_ALSACE"), "Alsace");
    }

    #[test]
    fn display_label_uses_localization() {
        let mut defs = GameDefs::default();
        defs.labels.insert("grain".into(), "Grain".into());
        assert_eq!(defs.display_label("grain"), "Grain");
    }
}
