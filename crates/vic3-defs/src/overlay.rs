//! Optional JSON overlays layered on top of loaded [`GameDefs`](crate::GameDefs).
//!
//! # Merge order
//!
//! Bottom → top (higher wins on conflict):
//!
//! 1. **Code defaults** — hardcoded model constants (e.g.
//!    [`crate::DEFAULT_PRICE_RANGE`]).
//! 2. **Blob / install** — postcard snapshot or Clausewitz parse via
//!    [`crate::load_from_path`] / [`crate::decode_blob`].
//! 3. **File overlays** — this module; applied last with
//!    [`apply_overlay`].
//!
//! Overlays are additive overrides: fields omitted from the JSON leave the
//! underlying [`GameDefs`](crate::GameDefs) unchanged. They do **not** bump
//! [`crate::BLOB_VERSION`]; the blob stays the install/extract layer.
//!
//! # Unknown keys
//!
//! Unknown JSON object keys are **ignored** (serde default). Building ids that
//! are not present in [`GameDefs::building_types`](crate::GameDefs::building_types) are
//! also ignored — overlays never invent new building types.
//!
//! # Examples
//!
//! ```
//! use vic3_defs::{apply_overlay, load_overlay_json, BuildingType, GameDefs};
//!
//! let mut defs = GameDefs::default();
//! defs.building_types.insert(
//!     "building_rye_farm".into(),
//!     BuildingType {
//!         id: "building_rye_farm".into(),
//!         group: None,
//!         city_type: None,
//!         production_method_groups: vec![],
//!         required_construction: Some(200.0),
//!     },
//! );
//!
//! let overlay = load_overlay_json(
//!     r#"{ "buildings": { "building_rye_farm": { "required_construction": 999.0 } } }"#,
//! )
//! .expect("valid overlay JSON");
//! apply_overlay(&mut defs, &overlay);
//! assert_eq!(
//!     defs.building_types["building_rye_farm"].required_construction,
//!     Some(999.0)
//! );
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{DefsError, GameDefs};

/// Parsed overlay document ready to merge onto [`GameDefs`].
///
/// Only fields present in the overlay participate in the merge. See the
/// [module docs](self) for merge order and unknown-key policy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DefsOverlay {
    /// Per-building overrides keyed by building type id (e.g. `building_rye_farm`).
    #[serde(default)]
    pub buildings: BTreeMap<String, BuildingOverlay>,
}

/// Fields that may be overridden on an existing [`crate::BuildingType`].
///
/// Omitted fields leave the building’s current values untouched.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BuildingOverlay {
    /// When set, replaces [`crate::BuildingType::required_construction`].
    #[serde(default)]
    pub required_construction: Option<f64>,
}

/// Deserialize a [`DefsOverlay`] from a JSON string.
///
/// Unknown object keys are ignored. Returns [`DefsError::OverlayJson`] when the
/// payload is not valid JSON or does not match the overlay shape.
pub fn load_overlay_json(json: &str) -> Result<DefsOverlay, DefsError> {
    serde_json::from_str(json).map_err(DefsError::OverlayJson)
}

/// Apply `overlay` onto `defs` in place.
///
/// For each building id in the overlay that already exists in
/// [`GameDefs::building_types`]:
/// - if [`BuildingOverlay::required_construction`] is [`Some`], that value
///   replaces the building’s construction cost;
/// - otherwise the field is left unchanged.
///
/// Building ids absent from `defs` are skipped. Buildings not mentioned in the
/// overlay are never touched.
pub fn apply_overlay(defs: &mut GameDefs, overlay: &DefsOverlay) {
    for (id, building_overlay) in &overlay.buildings {
        let Some(building) = defs.building_types.get_mut(id) else {
            continue;
        };
        if let Some(required_construction) = building_overlay.required_construction {
            building.required_construction = Some(required_construction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildingType;

    fn farm_defs(required_construction: Option<f64>) -> GameDefs {
        let mut defs = GameDefs::default();
        defs.building_types.insert(
            "building_rye_farm".into(),
            BuildingType {
                id: "building_rye_farm".into(),
                group: Some("bg_agriculture".into()),
                city_type: Some("farm".into()),
                production_method_groups: vec!["pmg_base_building_rye_farm".into()],
                required_construction,
            },
        );
        defs
    }

    #[test]
    fn apply_overlay_overrides_required_construction() {
        let mut defs = farm_defs(Some(200.0));
        let overlay = load_overlay_json(
            r#"{
                "buildings": {
                    "building_rye_farm": { "required_construction": 42.0 }
                }
            }"#,
        )
        .expect("overlay JSON");
        apply_overlay(&mut defs, &overlay);
        assert_eq!(
            defs.building_types["building_rye_farm"].required_construction,
            Some(42.0)
        );
    }

    #[test]
    fn without_overlay_required_construction_unchanged() {
        let defs = farm_defs(Some(200.0));
        assert_eq!(
            defs.building_types["building_rye_farm"].required_construction,
            Some(200.0)
        );
    }

    #[test]
    fn unknown_json_keys_and_building_ids_are_ignored() {
        let mut defs = farm_defs(Some(200.0));
        let overlay = load_overlay_json(
            r#"{
                "not_a_real_section": true,
                "buildings": {
                    "building_rye_farm": {
                        "required_construction": 77.0,
                        "extra_field": "ignored"
                    },
                    "building_does_not_exist": {
                        "required_construction": 1.0
                    }
                }
            }"#,
        )
        .expect("overlay JSON with unknowns");
        apply_overlay(&mut defs, &overlay);
        assert_eq!(
            defs.building_types["building_rye_farm"].required_construction,
            Some(77.0)
        );
        assert!(!defs.building_types.contains_key("building_does_not_exist"));
    }

    #[test]
    fn empty_building_entry_leaves_cost_unchanged() {
        let mut defs = farm_defs(Some(200.0));
        let overlay = load_overlay_json(r#"{ "buildings": { "building_rye_farm": {} } }"#)
            .expect("empty building overlay");
        apply_overlay(&mut defs, &overlay);
        assert_eq!(
            defs.building_types["building_rye_farm"].required_construction,
            Some(200.0)
        );
    }
}
