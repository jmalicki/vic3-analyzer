//! Army/navy power projection from units and staffed military buildings.
//!
//! Vic3 model: `unit_pp = ((offense + defense) / 2) * manpower_ratio`, summed
//! over units/ships. Underemployment cuts the manpower ratio. Post-1.13 navy
//! capacity is limited by both shipyards (hulls) and naval administrations
//! (crews).

use serde::{Deserialize, Serialize};

/// Standing army barracks (not conscription centers).
pub const BUILDING_BARRACKS: &str = "building_barracks";
/// Merged shipyards (1.13+) that produce ship construction capacity.
pub const BUILDING_SHIPYARD: &str = "building_shipyards";
/// Alternate shipyard script id seen in some defs.
pub const BUILDING_SHIPYARD_ALT: &str = "building_shipyard";
/// Naval administrations that crew ships (1.13+).
pub const BUILDING_NAVAL_ADMIN: &str = "building_naval_administration";
/// Construction sectors that raise national construction capacity.
pub const BUILDING_CONSTRUCTION_SECTOR: &str = "building_construction_sector";

/// Default infantry-like stats when combat-unit defs are absent.
pub const DEFAULT_ARMY_OFFENSE: f64 = 20.0;
pub const DEFAULT_ARMY_DEFENSE: f64 = 20.0;
/// Default light-ship-like stats when combat-unit defs are absent.
pub const DEFAULT_NAVY_OFFENSE: f64 = 25.0;
pub const DEFAULT_NAVY_DEFENSE: f64 = 25.0;
pub const DEFAULT_MAX_MANPOWER: f64 = 1000.0;

/// Staffing within this absolute level epsilon counts as full employment.
pub const STAFFING_EPS: f64 = 1e-6;

/// Model input-price factor: above `base_price * this`, queue producers.
pub const MIL_INPUT_PRICE_FACTOR: f64 = 1.25;

/// One military (or navy support) building aggregate on a planning branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeledMilBuilding {
    pub building: String,
    /// Built levels (capacity).
    pub levels: f64,
    /// Employed level-equivalents (same units as [`vic3_prices::WorldBuilding::staffing`]).
    pub staffing: f64,
}

impl Eq for ModeledMilBuilding {}

impl std::hash::Hash for ModeledMilBuilding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.building.hash(state);
        self.levels.to_bits().hash(state);
        self.staffing.to_bits().hash(state);
    }
}

impl ModeledMilBuilding {
    pub fn staffing_ratio(&self) -> f64 {
        if self.levels <= 0.0 {
            0.0
        } else {
            (self.staffing / self.levels).clamp(0.0, 1.0)
        }
    }

    pub fn is_fully_staffed(&self) -> bool {
        self.levels <= 0.0 || self.staffing + STAFFING_EPS >= self.levels
    }

    /// Employed capacity that can generate units/ships.
    pub fn effective_levels(&self) -> f64 {
        self.staffing.clamp(0.0, self.levels.max(0.0))
    }

    pub fn underemployed_levels(&self) -> f64 {
        (self.levels - self.staffing).max(0.0)
    }
}

/// Offense / defense / max manpower for one unit or ship type.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnitCombatStats {
    pub offense: f64,
    pub defense: f64,
    pub max_manpower: f64,
}

impl UnitCombatStats {
    pub fn army_default() -> Self {
        Self {
            offense: DEFAULT_ARMY_OFFENSE,
            defense: DEFAULT_ARMY_DEFENSE,
            max_manpower: DEFAULT_MAX_MANPOWER,
        }
    }

    pub fn navy_default() -> Self {
        Self {
            offense: DEFAULT_NAVY_OFFENSE,
            defense: DEFAULT_NAVY_DEFENSE,
            max_manpower: DEFAULT_MAX_MANPOWER,
        }
    }

    /// `unit_pp = ((offense + defense) / 2) * manpower_ratio`.
    pub fn power_projection(&self, manpower_ratio: f64) -> f64 {
        let ratio = manpower_ratio.clamp(0.0, 1.0);
        ((self.offense + self.defense) / 2.0) * ratio
    }

    pub fn full_power_projection(&self) -> f64 {
        self.power_projection(1.0)
    }
}

/// Whether a building type id is a shipyard (either script spelling).
pub fn is_shipyard_building(id: &str) -> bool {
    let key = building_key(id);
    key == building_key(BUILDING_SHIPYARD) || key == building_key(BUILDING_SHIPYARD_ALT)
}

pub fn is_barracks_building(id: &str) -> bool {
    building_key(id) == building_key(BUILDING_BARRACKS)
}

pub fn is_naval_admin_building(id: &str) -> bool {
    building_key(id) == building_key(BUILDING_NAVAL_ADMIN)
}

pub fn is_military_planning_building(id: &str) -> bool {
    is_barracks_building(id) || is_shipyard_building(id) || is_naval_admin_building(id)
}

fn building_key(id: &str) -> String {
    id.trim()
        .trim_start_matches("building_")
        .to_ascii_lowercase()
}

/// Army PP contributed by staffed barracks levels.
pub fn army_pp_from_buildings(buildings: &[ModeledMilBuilding], unit: UnitCombatStats) -> f64 {
    buildings
        .iter()
        .filter(|b| is_barracks_building(&b.building))
        .map(|b| b.effective_levels() * unit.full_power_projection())
        .sum()
}

/// Navy PP: ships limited by min(shipyard effective, naval-admin effective).
pub fn navy_pp_from_buildings(buildings: &[ModeledMilBuilding], unit: UnitCombatStats) -> f64 {
    let shipyard = buildings
        .iter()
        .filter(|b| is_shipyard_building(&b.building))
        .map(ModeledMilBuilding::effective_levels)
        .sum::<f64>();
    let admin = buildings
        .iter()
        .filter(|b| is_naval_admin_building(&b.building))
        .map(ModeledMilBuilding::effective_levels)
        .sum::<f64>();
    shipyard.min(admin) * unit.full_power_projection()
}

/// True when every barracks with levels > 0 is fully staffed.
pub fn army_buildings_fully_staffed(buildings: &[ModeledMilBuilding]) -> bool {
    buildings
        .iter()
        .filter(|b| is_barracks_building(&b.building) && b.levels > 0.0)
        .all(ModeledMilBuilding::is_fully_staffed)
}

/// True when every shipyard / naval-admin with levels > 0 is fully staffed.
pub fn navy_buildings_fully_staffed(buildings: &[ModeledMilBuilding]) -> bool {
    buildings
        .iter()
        .filter(|b| {
            (is_shipyard_building(&b.building) || is_naval_admin_building(&b.building))
                && b.levels > 0.0
        })
        .all(ModeledMilBuilding::is_fully_staffed)
}

/// True when every barracks/shipyard/naval-admin with levels > 0 is fully staffed.
pub fn military_buildings_fully_staffed(buildings: &[ModeledMilBuilding]) -> bool {
    army_buildings_fully_staffed(buildings) && navy_buildings_fully_staffed(buildings)
}

/// Recompute army PP = baseline + delta from modeled barracks (when baseline known).
pub fn recompute_army_pp(
    baseline: Option<f64>,
    buildings: &[ModeledMilBuilding],
    unit: UnitCombatStats,
) -> Option<f64> {
    let baseline = baseline?;
    Some(baseline + army_pp_from_buildings(buildings, unit))
}

/// Recompute navy PP = baseline + delta from shipyard∩admin capacity.
pub fn recompute_navy_pp(
    baseline: Option<f64>,
    buildings: &[ModeledMilBuilding],
    unit: UnitCombatStats,
) -> Option<f64> {
    let baseline = baseline?;
    Some(baseline + navy_pp_from_buildings(buildings, unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_formula_matches_wiki() {
        let stats = UnitCombatStats {
            offense: 30.0,
            defense: 10.0,
            max_manpower: 1000.0,
        };
        assert!((stats.power_projection(1.0) - 20.0).abs() < 1e-9);
        assert!((stats.power_projection(0.5) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn underemployed_barracks_cut_army_pp() {
        let full = ModeledMilBuilding {
            building: BUILDING_BARRACKS.into(),
            levels: 2.0,
            staffing: 2.0,
        };
        let half = ModeledMilBuilding {
            building: BUILDING_BARRACKS.into(),
            levels: 2.0,
            staffing: 1.0,
        };
        let unit = UnitCombatStats::army_default();
        assert!(
            (army_pp_from_buildings(&[full], unit) - 2.0 * unit.full_power_projection()).abs()
                < 1e-9
        );
        assert!(
            (army_pp_from_buildings(&[half], unit) - unit.full_power_projection()).abs() < 1e-9
        );
    }

    #[test]
    fn navy_limited_by_min_shipyard_and_admin() {
        let buildings = vec![
            ModeledMilBuilding {
                building: BUILDING_SHIPYARD.into(),
                levels: 4.0,
                staffing: 4.0,
            },
            ModeledMilBuilding {
                building: BUILDING_NAVAL_ADMIN.into(),
                levels: 2.0,
                staffing: 2.0,
            },
        ];
        let unit = UnitCombatStats::navy_default();
        assert!(
            (navy_pp_from_buildings(&buildings, unit) - 2.0 * unit.full_power_projection()).abs()
                < 1e-9
        );
    }

    #[test]
    fn underemployed_blocks_full_staffed_check() {
        let buildings = vec![ModeledMilBuilding {
            building: BUILDING_BARRACKS.into(),
            levels: 1.0,
            staffing: 0.5,
        }];
        assert!(!military_buildings_fully_staffed(&buildings));
        assert!(!buildings[0].is_fully_staffed());
    }
}
