//! Synthetic (and later IR-backed) market: pops, buildings, frozen orders.

use std::collections::BTreeMap;

use vic3_defs::{GameDefs, ProductionMethod};
use vic3_load::{Building, Pop, Save, TradeRoute};

/// Pop size unit for buy packages (Vic3: package values are per 10k working pops).
pub const POP_SCALE: f64 = 10_000.0;

/// Market snapshot owned by this crate. Can be filled from `vic3-load` IR later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct World {
    pub states: Vec<WorldState>,
    pub pops: Vec<WorldPop>,
    pub buildings: Vec<WorldBuilding>,
    /// Government / trade / construction buy orders, held fixed during the solve.
    pub frozen_buy: BTreeMap<String, f64>,
    /// Trade (and any other non-building) sell orders, held fixed during the solve.
    pub frozen_sell: BTreeMap<String, f64>,
    /// Save pops dropped for missing `size` or `wealth`. They consume nothing,
    /// so a large count here explains a market stuck at base prices.
    pub skipped_pops: usize,
    /// Save buildings dropped for a missing type id.
    pub skipped_buildings: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldState {
    pub id: u32,
    pub region: Option<String>,
    pub country: Option<u32>,
    pub market: Option<u32>,
}

/// A pop whose consumption sits in the price loop.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldPop {
    pub state: Option<u32>,
    pub size: f64,
    /// Saved wealth 1–99. Used as the Laspeyres reference basket.
    pub wealth: u8,
    /// Frozen wage bill. When `≤ 0`, wealth stays at [`Self::wealth`].
    pub wages: f64,
    pub culture: Option<String>,
}

/// A building whose goods IO is reconstructed from defs PMs and then frozen
/// (employment = [`Self::staffing`] does not change in [`crate::what_if`]).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldBuilding {
    pub id: u32,
    pub state: Option<u32>,
    pub building: String,
    pub level: f64,
    /// Employment / throughput fraction. Frozen except that what-if does not touch it.
    pub staffing: f64,
    /// Active production methods, one per PM group; a building runs them all.
    pub production_methods: Vec<String>,
}

/// Non-pop buy and sell orders: frozen maps plus building PM inputs/outputs.
///
/// Building volumes use `level * staffing` and current PM recipes. They are
/// held fixed for a given [`World`]; [`crate::what_if`] clones the world and
/// bumps `level` only.
pub fn reconstruct_non_pop_orders(
    world: &World,
    defs: &GameDefs,
) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
    let mut buy = world.frozen_buy.clone();
    let mut sell = world.frozen_sell.clone();
    for building in &world.buildings {
        let scale = building.level * building.staffing;
        if scale == 0.0 {
            continue;
        }
        for pm in building.methods(defs) {
            for (good, qty) in &pm.inputs {
                *buy.entry(good.clone()).or_default() += *qty * scale;
            }
            for (good, qty) in &pm.outputs {
                *sell.entry(good.clone()).or_default() += *qty * scale;
            }
        }
    }
    (buy, sell)
}

impl World {
    /// Frozen market snapshot from save IR.
    ///
    /// Pops missing `size`/`wealth`, buildings with an empty type id, and trade
    /// routes missing goods, volume, or export direction are skipped.
    pub fn from_save(save: &Save) -> Self {
        let states = save
            .states
            .iter_present()
            .map(|(id, state)| WorldState {
                id,
                region: state.region.clone(),
                country: state.country,
                market: state.market,
            })
            .collect();
        let saved_pops = save.pops.iter_present().count();
        let pops: Vec<_> = save
            .pops
            .iter_present()
            .filter_map(|(_, pop)| WorldPop::from_ir(pop))
            .collect();
        let saved_buildings = save.building_manager.iter_present().count();
        let buildings: Vec<_> = save
            .building_manager
            .iter_present()
            .filter_map(|(id, building)| WorldBuilding::from_ir(id, building))
            .collect();
        let mut frozen_buy = BTreeMap::new();
        let mut frozen_sell = BTreeMap::new();
        for (_, route) in save.trade_route_manager.iter_present() {
            apply_trade_route(route, &mut frozen_buy, &mut frozen_sell);
        }
        Self {
            skipped_pops: saved_pops - pops.len(),
            skipped_buildings: saved_buildings - buildings.len(),
            states,
            pops,
            buildings,
            frozen_buy,
            frozen_sell,
        }
    }

    /// Clone this world and add `extra_levels` to every building of `building` type.
    ///
    /// Staffing (employment) is left unchanged.
    pub fn with_extra_levels(&self, building: &str, extra_levels: u32) -> Self {
        let mut next = self.clone();
        let extra = f64::from(extra_levels);
        for b in &mut next.buildings {
            if b.building == building {
                b.level += extra;
            }
        }
        next
    }
}

impl WorldPop {
    fn from_ir(pop: &Pop) -> Option<Self> {
        let size = pop.size.filter(|s| *s > 0.0)?;
        let wealth = pop.wealth?;
        let wealth = u8::try_from(wealth.clamp(1, 99)).ok()?;
        Some(Self {
            state: pop.state,
            size,
            wealth,
            wages: pop.wages.filter(|w| *w > 0.0).unwrap_or(0.0),
            culture: pop.culture.clone(),
        })
    }
}

impl WorldBuilding {
    fn from_ir(id: u32, building: &Building) -> Option<Self> {
        if building.building.is_empty() {
            return None;
        }
        Some(Self {
            id,
            state: building.state,
            building: building.building.clone(),
            level: f64::from(building.level.max(0)),
            staffing: building.staffing.max(0.0),
            production_methods: building.active_production_methods(),
        })
    }

    /// Production methods this building runs that the definitions describe.
    pub fn methods<'a>(&self, defs: &'a GameDefs) -> Vec<&'a ProductionMethod> {
        self.production_methods
            .iter()
            .filter_map(|id| defs.production_methods.get(id))
            .collect()
    }

    /// A building whose methods are all unknown produces and consumes nothing,
    /// which would otherwise look like a real, balanced market.
    pub fn has_known_method(&self, defs: &GameDefs) -> bool {
        !self.methods(defs).is_empty()
    }
}

fn apply_trade_route(
    route: &TradeRoute,
    frozen_buy: &mut BTreeMap<String, f64>,
    frozen_sell: &mut BTreeMap<String, f64>,
) {
    let Some(good) = route.goods.as_ref().filter(|g| !g.is_empty()) else {
        return;
    };
    let Some(volume) = route.volume.filter(|v| *v != 0.0) else {
        return;
    };
    let Some(export) = route.export else {
        return;
    };
    let dest = if export { frozen_sell } else { frozen_buy };
    *dest.entry(good.clone()).or_default() += volume.abs();
}

#[cfg(test)]
mod tests {
    use super::*;
    use vic3_load::{Building, Pop, Save, TradeRoute};

    #[test]
    fn from_save_skips_missing_pop_and_building_fields() {
        let mut save = Save::default();
        save.pops.database.insert(
            1,
            Some(Pop {
                size: None,
                wealth: Some(8),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            2,
            Some(Pop {
                size: Some(1_000.0),
                wealth: None,
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            3,
            Some(Pop {
                size: Some(10_000.0),
                wealth: Some(8),
                culture: Some("north_german".into()),
                ..Pop::default()
            }),
        );
        save.building_manager.database.insert(
            1,
            Some(Building {
                building: String::new(),
                level: 2,
                staffing: 1.0,
                ..Building::default()
            }),
        );
        save.building_manager.database.insert(
            2,
            Some(Building {
                building: "building_rye_farm".into(),
                level: 2,
                staffing: 1.0,
                production_method: Some("pm_simple_farming".into()),
                ..Building::default()
            }),
        );
        save.trade_route_manager.database.insert(
            1,
            Some(TradeRoute {
                goods: Some("grain".into()),
                volume: Some(50.0),
                export: None,
            }),
        );
        save.trade_route_manager.database.insert(
            2,
            Some(TradeRoute {
                goods: Some("wood".into()),
                volume: Some(10.0),
                export: Some(true),
            }),
        );

        let world = World::from_save(&save);
        assert_eq!(world.pops.len(), 1);
        assert_eq!(world.pops[0].size, 10_000.0);
        assert_eq!(world.pops[0].wealth, 8);
        assert_eq!(world.buildings.len(), 1);
        assert_eq!(world.buildings[0].building, "building_rye_farm");
        assert_eq!(world.buildings[0].level, 2.0);
        assert_eq!(world.buildings[0].production_methods, ["pm_simple_farming"]);
        assert!(world.frozen_buy.is_empty());
        assert_eq!(world.frozen_sell.get("wood").copied(), Some(10.0));
        assert!(!world.frozen_sell.contains_key("grain"));
        assert_eq!(world.skipped_pops, 2);
        assert_eq!(world.skipped_buildings, 1);
    }

    /// A real save lists one active method per PM group; a building runs them all.
    #[test]
    fn from_save_reads_the_plural_production_method_list() {
        let mut save = Save::default();
        save.building_manager.database.insert(
            1,
            Some(Building {
                building: "building_rye_farm".into(),
                level: 2,
                staffing: 1.0,
                production_methods: vec![
                    "pm_simple_farming".into(),
                    "pm_no_automation".into(),
                    String::new(),
                ],
                ..Building::default()
            }),
        );

        let world = World::from_save(&save);
        assert_eq!(
            world.buildings[0].production_methods,
            ["pm_simple_farming", "pm_no_automation"]
        );
    }

    #[test]
    fn all_active_methods_place_orders() {
        let defs = GameDefs {
            production_methods: BTreeMap::from([
                (
                    "pm_smithy".into(),
                    ProductionMethod {
                        id: "pm_smithy".into(),
                        inputs: BTreeMap::from([("iron".into(), 2.0)]),
                        outputs: BTreeMap::from([("tools".into(), 3.0)]),
                    },
                ),
                (
                    "pm_steam".into(),
                    ProductionMethod {
                        id: "pm_steam".into(),
                        inputs: BTreeMap::from([("coal".into(), 5.0), ("iron".into(), 1.0)]),
                        outputs: BTreeMap::new(),
                    },
                ),
            ]),
            ..GameDefs::default()
        };
        let world = World {
            buildings: vec![WorldBuilding {
                id: 1,
                state: None,
                building: "building_tooling_workshops".into(),
                level: 2.0,
                staffing: 1.0,
                production_methods: vec!["pm_smithy".into(), "pm_steam".into()],
            }],
            ..World::default()
        };

        let (buy, sell) = reconstruct_non_pop_orders(&world, &defs);
        assert_eq!(buy.get("iron").copied(), Some(6.0));
        assert_eq!(buy.get("coal").copied(), Some(10.0));
        assert_eq!(sell.get("tools").copied(), Some(6.0));
    }

    #[test]
    fn from_save_plaintext_fixture() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-load/tests/fixtures/plaintext.txt");
        let save = vic3_load::load_path(&path, vic3_load::empty_tokens()).expect("fixture");
        let world = World::from_save(&save);
        assert_eq!(world.pops.len(), 1);
        assert_eq!(world.pops[0].wealth, 8);
        assert_eq!(world.buildings.len(), 1);
        assert_eq!(world.buildings[0].building, "building_rye_farm");
        assert!(
            world.frozen_buy.is_empty() && world.frozen_sell.is_empty(),
            "fixture trade route has no export direction"
        );
    }
}
