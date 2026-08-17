//! Synthetic (and later IR-backed) market: pops, buildings, frozen orders.

use vic3_defs::{GameDefs, GoodIdx, GoodsVec, ProductionMethod};
use vic3_load::{Building, Pop, Save};

/// Pop size unit for buy packages (Vic3: package values are per 10k working pops).
pub const POP_SCALE: f64 = 10_000.0;

/// Market snapshot owned by this crate. Can be filled from `vic3-load` IR later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct World {
    pub states: Vec<WorldState>,
    pub countries: Vec<WorldCountry>,
    pub pops: Vec<WorldPop>,
    /// All save pops retained for state detail, including rows that could not
    /// enter the consumption model.
    pub state_pops: Vec<WorldStatePop>,
    pub buildings: Vec<WorldBuilding>,
    /// Government / trade / construction buy orders, held fixed during the solve.
    pub frozen_buy: GoodsVec,
    /// Trade (and any other non-building) sell orders, held fixed during the solve.
    pub frozen_sell: GoodsVec,
    /// Post-1.9 world-market goods volumes attributed to the state containing
    /// the trade center. Positive quantities are imports; negative quantities
    /// are exports.
    pub state_trade: Vec<WorldStateTrade>,
    /// Save pops dropped for missing household population (or legacy `size`) or
    /// `wealth`. They consume nothing, so a large count here explains a market
    /// stuck at base prices.
    pub skipped_pops: usize,
    /// Save buildings dropped for a missing type id.
    pub skipped_buildings: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldCountry {
    pub id: u32,
    pub tag: String,
    pub laws: Vec<String>,
    pub overlord: Option<u32>,
    pub subject_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldState {
    pub id: u32,
    pub region: Option<String>,
    pub country: Option<u32>,
    pub market: Option<u32>,
    pub arable_land: Option<f64>,
    pub infrastructure: Option<f64>,
    pub infrastructure_usage: Option<f64>,
    pub qualifications: std::collections::BTreeMap<String, f64>,
    pub employable_qualifications: std::collections::BTreeMap<String, f64>,
    pub workforce_by_type: std::collections::BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldStateTrade {
    pub state: u32,
    pub good: GoodIdx,
    /// Positive = import into the state; negative = export from the state.
    pub quantity: f64,
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
    pub profession: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldStatePop {
    pub id: u32,
    pub state: Option<u32>,
    pub demand_size: Option<f64>,
    pub workforce: Option<f64>,
    pub dependents: Option<f64>,
    pub wealth: Option<i32>,
    pub wages: Option<f64>,
    pub culture: Option<String>,
    pub profession: Option<String>,
    pub literate: Option<f64>,
    pub workplace_id: Option<u32>,
    pub qualifications: std::collections::BTreeMap<String, f64>,
}

/// A building whose goods IO is reconstructed from defs PMs and then frozen
/// (employment = [`Self::staffing`] does not change in [`crate::what_if`]).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldBuilding {
    pub id: u32,
    pub state: Option<u32>,
    pub building: String,
    pub level: f64,
    /// Staffed levels. Frozen except that what-if does not touch it.
    pub staffing: f64,
    /// Active production methods, one per PM group; a building runs them all.
    pub production_methods: Vec<String>,
    /// Absolute saved input volumes, resolved once against `goods_order`.
    pub saved_inputs: Vec<(GoodIdx, f64)>,
    /// Absolute saved output volumes, resolved once against `goods_order`.
    pub saved_outputs: Vec<(GoodIdx, f64)>,
}

/// Non-pop buy and sell orders: frozen maps plus building inputs/outputs.
///
/// Prefer absolute volumes saved on the building (`input_goods` /
/// `output_goods`). Fall back to production-method recipes only when the save
/// has no IO, scaling them by staffed levels.
pub fn reconstruct_non_pop_orders(world: &World, defs: &GameDefs) -> (GoodsVec, GoodsVec) {
    let mut buy = world.frozen_buy.aligned(defs.goods_order.len());
    let mut sell = world.frozen_sell.aligned(defs.goods_order.len());
    for trade in &world.state_trade {
        trade.add_orders(&mut buy, &mut sell, 1.0);
    }
    for building in &world.buildings {
        let (inputs, outputs) = building.goods_io(defs);
        for (good, qty) in inputs.iter_indexed() {
            buy.add(good, qty);
        }
        for (good, qty) in outputs.iter_indexed() {
            sell.add(good, qty);
        }
    }
    (buy, sell)
}

impl World {
    /// Frozen market snapshot from save IR.
    ///
    /// Pops missing household population (or legacy `size`) or `wealth`,
    /// buildings with an empty type id, and state trade entries with unknown
    /// goods-table indices are skipped.
    pub fn from_save(save: &Save, defs: &GameDefs) -> Self {
        let countries = save
            .country_manager
            .iter_present()
            .map(|(id, country)| WorldCountry {
                id,
                tag: country.definition.clone(),
                laws: save
                    .active_laws(id)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                overlord: country.overlord,
                subject_type: country.subject_type.clone(),
            })
            .collect();
        let states = save
            .states
            .iter_present()
            .map(|(id, state)| WorldState {
                id,
                region: state.region.clone(),
                country: state.country,
                market: state
                    .country
                    .and_then(|country_id| {
                        save.country_manager
                            .database
                            .get(&country_id)
                            .and_then(Option::as_ref)
                            .and_then(|country| country.market)
                    })
                    .or(state.market),
                arable_land: state.arable_land,
                infrastructure: state.infrastructure,
                infrastructure_usage: state.infrastructure_usage,
                qualifications: resolve_qty_map(&state.qualifications.values),
                employable_qualifications: resolve_qty_map(&state.employable().values),
                workforce_by_type: resolve_qty_map(&state.workforce_by_profession().values),
            })
            .collect();
        let saved_pops = save.pops.iter_present().count();
        let state_pops = save
            .pops
            .iter_present()
            .map(|(id, pop)| WorldStatePop {
                id,
                state: pop.state,
                demand_size: pop.demand_size(),
                workforce: pop.workforce,
                dependents: pop.dependents,
                wealth: pop.wealth,
                wages: pop.wages,
                culture: save.culture_id(pop.culture.as_deref()),
                profession: pop.profession.clone(),
                literate: pop.literate,
                workplace_id: pop.workplace,
                qualifications: resolve_qty_map(&pop.qualifications.values),
            })
            .collect();
        let pops: Vec<_> = save
            .pops
            .iter_present()
            .filter_map(|(_, pop)| {
                WorldPop::from_ir(pop).map(|mut world_pop| {
                    world_pop.culture = save.culture_id(pop.culture.as_deref());
                    world_pop
                })
            })
            .collect();
        let saved_buildings = save.building_manager.iter_present().count();
        let buildings: Vec<_> = save
            .building_manager
            .iter_present()
            .filter_map(|(id, building)| WorldBuilding::from_ir(id, building, defs))
            .collect();
        let state_trade = save
            .states
            .iter_present()
            .flat_map(|(state_id, state)| {
                resolve_saved_goods(&state.trade.goods, defs)
                    .into_iter()
                    .map(move |(good, quantity)| WorldStateTrade {
                        state: state_id,
                        good,
                        quantity: quantity
                            * defs
                                .traded_quantity_idx(good)
                                .unwrap_or(vic3_defs::DEFAULT_TRADED_QUANTITY),
                    })
            })
            .collect();
        Self {
            skipped_pops: saved_pops - pops.len(),
            skipped_buildings: saved_buildings - buildings.len(),
            countries,
            states,
            pops,
            state_pops,
            buildings,
            frozen_buy: GoodsVec::zeros(defs.goods_order.len()),
            frozen_sell: GoodsVec::zeros(defs.goods_order.len()),
            state_trade,
        }
    }

    /// Clone this world and add `extra_levels` to every building of `building` type.
    ///
    /// The saved staffing ratio and absolute saved goods IO are held constant
    /// per level, so explicit level additions scale both proportionally. Other
    /// employment, wages, and trade volumes remain frozen.
    pub fn with_extra_levels(&self, building: &str, extra_levels: u32) -> Self {
        let mut next = self.clone();
        let extra = f64::from(extra_levels);
        for b in &mut next.buildings {
            if b.building == building {
                let old_level = b.level.max(0.0);
                let new_level = old_level + extra;
                if extra > 0.0 {
                    if old_level > 0.0 {
                        let ratio = new_level / old_level;
                        b.staffing *= ratio;
                        for quantity in b
                            .saved_inputs
                            .iter_mut()
                            .map(|(_, quantity)| quantity)
                            .chain(b.saved_outputs.iter_mut().map(|(_, quantity)| quantity))
                        {
                            *quantity *= ratio;
                        }
                    } else {
                        // Synthetic/empty buildings have no saved per-level
                        // ratio; added capacity starts fully staffed.
                        b.staffing = new_level;
                    }
                }
                b.level = new_level;
            }
        }
        next
    }
}

impl WorldPop {
    fn from_ir(pop: &Pop) -> Option<Self> {
        let size = pop.demand_size()?;
        let wealth = pop.wealth?;
        let wealth = u8::try_from(wealth.clamp(1, 99)).ok()?;
        Some(Self {
            state: pop.state,
            size,
            wealth,
            wages: pop.wages.filter(|w| *w > 0.0).unwrap_or(0.0),
            culture: pop.culture.clone(),
            profession: pop.profession.clone(),
        })
    }
}

impl WorldBuilding {
    fn from_ir(id: u32, building: &Building, defs: &GameDefs) -> Option<Self> {
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
            saved_inputs: resolve_saved_goods(&building.input_goods.goods, defs),
            saved_outputs: resolve_saved_goods(&building.output_goods.goods, defs),
        })
    }

    /// Production methods this building runs that the definitions describe.
    pub fn methods<'a>(&self, defs: &'a GameDefs) -> Vec<&'a ProductionMethod> {
        self.production_methods
            .iter()
            .filter_map(|id| defs.production_methods.get(id))
            .collect()
    }

    /// Effective building IO. Saved current volumes are authoritative; PM
    /// recipes are used only when both saved sides are absent.
    pub fn goods_io(&self, defs: &GameDefs) -> (GoodsVec, GoodsVec) {
        if !self.saved_inputs.is_empty() || !self.saved_outputs.is_empty() {
            let mut inputs = GoodsVec::zeros(defs.goods_order.len());
            let mut outputs = GoodsVec::zeros(defs.goods_order.len());
            for &(good, qty) in &self.saved_inputs {
                inputs.add(good, qty);
            }
            for &(good, qty) in &self.saved_outputs {
                outputs.add(good, qty);
            }
            return (inputs, outputs);
        }

        let scale = self.staffed_levels();
        let mut inputs = GoodsVec::zeros(defs.goods_order.len());
        let mut outputs = GoodsVec::zeros(defs.goods_order.len());
        for method in self.methods(defs) {
            for (good, qty) in &method.inputs {
                inputs.add(*good, *qty * scale);
            }
            for (good, qty) in &method.outputs {
                outputs.add(*good, *qty * scale);
            }
        }
        (inputs, outputs)
    }

    /// Throughput in level units. Real saves store `staffing` in the same unit
    /// as `levels`, not as a fraction.
    pub fn staffed_levels(&self) -> f64 {
        self.staffing.clamp(0.0, self.level.max(0.0))
    }

    /// True when the building can place goods orders from PMs or saved IO.
    pub fn has_known_method(&self, defs: &GameDefs) -> bool {
        !self.methods(defs).is_empty()
            || !self.saved_inputs.is_empty()
            || !self.saved_outputs.is_empty()
    }

    /// True when the effective IO contains a non-zero order.
    pub fn has_orders(&self, defs: &GameDefs) -> bool {
        let (inputs, outputs) = self.goods_io(defs);
        inputs
            .as_slice()
            .iter()
            .chain(outputs.as_slice())
            .any(|quantity| quantity.abs() > crate::ORDER_EPS)
    }
}

impl WorldStateTrade {
    pub(crate) fn add_orders(&self, buy: &mut GoodsVec, sell: &mut GoodsVec, scale: f64) {
        let quantity = self.quantity * scale;
        if quantity > 0.0 {
            sell.add(self.good, quantity);
        } else if quantity < 0.0 {
            buy.add(self.good, -quantity);
        }
    }
}

/// Vanilla `common/pop_types` filename order. Wiki 1.13: index 0 is academics.
pub const VANILLA_POP_TYPES: &[&str] = &[
    "academics",
    "aristocrats",
    "bureaucrats",
    "capitalists",
    "clergymen",
    "clerks",
    "engineers",
    "farmers",
    "laborers",
    "machinists",
    "officers",
    "peasants",
    "shopkeepers",
    "slaves",
    "soldiers",
];

pub(crate) fn resolve_profession_key(key: &str) -> String {
    if let Ok(index) = key.parse::<usize>() {
        if let Some(id) = VANILLA_POP_TYPES.get(index) {
            return (*id).to_string();
        }
    }
    key.to_string()
}

fn resolve_qty_map(
    raw: &std::collections::BTreeMap<String, f64>,
) -> std::collections::BTreeMap<String, f64> {
    let mut out = std::collections::BTreeMap::new();
    for (key, qty) in raw {
        *out.entry(resolve_profession_key(key)).or_default() += *qty;
    }
    out
}

fn resolve_saved_goods(
    raw: &std::collections::BTreeMap<String, f64>,
    defs: &GameDefs,
) -> Vec<(GoodIdx, f64)> {
    raw.iter()
        .filter_map(|(key, quantity)| {
            let good = match key.parse::<usize>() {
                Ok(index) => {
                    let idx = GoodIdx::try_from_usize(index)?;
                    defs.good_by_index(idx)?;
                    idx
                }
                Err(_) => defs.index_of(key)?,
            };
            Some((good, *quantity))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vic3_load::{Building, BuildingGoods, Pop, Save, State};

    fn defs_with_goods(ids: &[&str]) -> GameDefs {
        GameDefs {
            goods_order: ids.iter().map(|id| (*id).to_string()).collect(),
            goods: ids
                .iter()
                .map(|id| {
                    (
                        (*id).to_string(),
                        vic3_defs::Good {
                            id: (*id).to_string(),
                            base_price: 20.0,
                            traded_quantity: 10.0,
                            texture: None,
                        },
                    )
                })
                .collect(),
            ..GameDefs::default()
        }
    }

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
        save.pops.database.insert(
            4,
            Some(Pop {
                workforce: Some(10_000.0),
                dependents: Some(0.0),
                wealth: Some(8),
                culture: Some("weighted_size".into()),
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
        save.states.database.insert(
            1,
            Some(State {
                trade: BuildingGoods {
                    goods: BTreeMap::from([
                        ("grain".into(), 50.0),
                        ("wood".into(), -10.0),
                        ("unknown".into(), 99.0),
                    ]),
                },
                ..State::default()
            }),
        );

        let mut defs = defs_with_goods(&["grain", "wood"]);
        defs.goods
            .get_mut("grain")
            .expect("grain definition")
            .traded_quantity = 12.0;
        let world = World::from_save(&save, &defs);
        assert_eq!(world.pops.len(), 2);
        let weighted_pop = world
            .pops
            .iter()
            .find(|pop| pop.culture.as_deref() == Some("weighted_size"))
            .expect("pop with split size fields");
        assert_eq!(weighted_pop.size, 10_000.0);
        assert_eq!(weighted_pop.wealth, 8);
        assert_eq!(world.buildings.len(), 1);
        assert_eq!(world.buildings[0].building, "building_rye_farm");
        assert_eq!(world.buildings[0].level, 2.0);
        assert_eq!(world.buildings[0].production_methods, ["pm_simple_farming"]);
        assert_eq!(world.frozen_buy.as_slice(), &[0.0, 0.0]);
        assert_eq!(world.frozen_sell.as_slice(), &[0.0, 0.0]);
        assert_eq!(world.state_trade.len(), 2);
        let (buy, sell) = reconstruct_non_pop_orders(&world, &defs);
        assert_eq!(buy[defs.index_of("wood").unwrap()], 100.0);
        assert_eq!(sell[defs.index_of("grain").unwrap()], 600.0);
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

        let defs = defs_with_goods(&[]);
        let world = World::from_save(&save, &defs);
        assert_eq!(
            world.buildings[0].production_methods,
            ["pm_simple_farming", "pm_no_automation"]
        );
    }

    #[test]
    fn saved_building_io_places_orders_when_pm_unknown() {
        let mut save = Save::default();
        save.building_manager.database.insert(
            1,
            Some(Building {
                building: "building_logging_camp".into(),
                level: 2,
                staffing: 1.0,
                production_methods: vec!["pm_unknown_modded".into()],
                output_goods: BuildingGoods {
                    goods: BTreeMap::from([("wood".into(), 40.0)]),
                },
                input_goods: BuildingGoods {
                    goods: BTreeMap::from([("tools".into(), 2.0)]),
                },
                ..Building::default()
            }),
        );
        let defs = defs_with_goods(&["wood", "tools"]);
        let world = World::from_save(&save, &defs);
        let (buy, sell) = reconstruct_non_pop_orders(&world, &defs);
        assert_eq!(buy[defs.index_of("tools").unwrap()], 2.0);
        assert_eq!(sell[defs.index_of("wood").unwrap()], 40.0);
        let result = crate::solve(&world, &defs, crate::SolveOpts::default());
        let wood = result.goods.iter().find(|g| g.id == "wood").expect("wood");
        assert!(wood.price < wood.base);
        assert!(result.inputs.goods_with_orders > 0);
    }

    #[test]
    fn extra_levels_scale_saved_io_and_staffing_ratio() {
        let tools = GoodIdx::from_usize(0);
        let wood = GoodIdx::from_usize(1);
        let world = World {
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(7),
                building: "building_logging_camp".into(),
                level: 2.0,
                staffing: 1.0,
                production_methods: vec!["pm_unknown_modded".into()],
                saved_inputs: vec![(tools, 2.0)],
                saved_outputs: vec![(wood, 40.0)],
            }],
            ..World::default()
        };

        let bumped = world.with_extra_levels("building_logging_camp", 2);
        assert_eq!(world.buildings[0].level, 2.0, "source world is immutable");
        assert_eq!(world.buildings[0].staffing, 1.0);
        assert_eq!(bumped.buildings[0].level, 4.0);
        assert_eq!(bumped.buildings[0].staffing, 2.0);
        assert_eq!(bumped.buildings[0].saved_inputs, [(tools, 4.0)]);
        assert_eq!(bumped.buildings[0].saved_outputs, [(wood, 80.0)]);
    }

    #[test]
    fn all_active_methods_place_orders() {
        let mut defs = defs_with_goods(&["iron", "tools", "coal"]);
        defs.production_methods = BTreeMap::from([
            (
                "pm_smithy".into(),
                ProductionMethod {
                    id: "pm_smithy".into(),
                    inputs: vec![(GoodIdx::from_usize(0), 2.0)],
                    outputs: vec![(GoodIdx::from_usize(1), 3.0)],
                },
            ),
            (
                "pm_steam".into(),
                ProductionMethod {
                    id: "pm_steam".into(),
                    inputs: vec![(GoodIdx::from_usize(2), 5.0), (GoodIdx::from_usize(0), 1.0)],
                    outputs: Vec::new(),
                },
            ),
        ]);
        let world = World {
            buildings: vec![WorldBuilding {
                id: 1,
                state: None,
                building: "building_tooling_workshops".into(),
                level: 2.0,
                staffing: 2.0,
                production_methods: vec!["pm_smithy".into(), "pm_steam".into()],
                saved_inputs: Default::default(),
                saved_outputs: Default::default(),
            }],
            ..World::default()
        };

        let (buy, sell) = reconstruct_non_pop_orders(&world, &defs);
        assert_eq!(buy[defs.index_of("iron").unwrap()], 6.0);
        assert_eq!(buy[defs.index_of("coal").unwrap()], 10.0);
        assert_eq!(sell[defs.index_of("tools").unwrap()], 6.0);
    }

    #[test]
    fn saved_integer_io_overrides_pm_recipes() {
        let mut defs = defs_with_goods(&["merchant_marine", "iron"]);
        defs.production_methods = BTreeMap::from([(
            "pm_mine".into(),
            ProductionMethod {
                id: "pm_mine".into(),
                inputs: vec![(GoodIdx::from_usize(0), 10.0)],
                outputs: vec![(GoodIdx::from_usize(1), 20.0)],
            },
        )]);
        let world = World {
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_iron_mine".into(),
                level: 10.0,
                staffing: 5.0,
                production_methods: vec!["pm_mine".into()],
                saved_inputs: vec![(GoodIdx::from_usize(0), 32.5)],
                saved_outputs: vec![(GoodIdx::from_usize(1), 130.0)],
            }],
            ..World::default()
        };

        let (buy, sell) = reconstruct_non_pop_orders(&world, &defs);
        assert_eq!(buy.as_slice(), &[32.5, 0.0]);
        assert_eq!(sell.as_slice(), &[0.0, 130.0]);
    }

    #[test]
    fn from_save_plaintext_fixture() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-load/tests/fixtures/plaintext.txt");
        let save = vic3_load::load_path(&path, vic3_load::empty_tokens()).expect("fixture");
        let defs_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-defs/tests/fixtures");
        let defs = vic3_defs::load_from_path(defs_root).expect("defs fixture");
        let world = World::from_save(&save, &defs);
        assert_eq!(world.pops.len(), 1);
        assert_eq!(world.pops[0].wealth, 8);
        assert_eq!(world.states[0].market, Some(1));
        assert_eq!(world.countries[0].laws, ["law_autocracy"]);
        assert_eq!(world.buildings.len(), 1);
        assert_eq!(world.buildings[0].building, "building_rye_farm");
        let result = crate::solve(&world, &defs, crate::SolveOpts::default());
        assert!(result.inputs.goods_with_orders > 0);
        assert_eq!(result.inputs.buildings_without_orders, 0);
        assert!(!result.buildings.is_empty());
        assert_eq!(result.states[0].arable_land, Some(45.0));
        assert_eq!(result.states[0].infrastructure, Some(32.5));
        assert_eq!(result.state_pops.len(), 1);
        assert_eq!(
            result.state_pops[0].profession_id.as_deref(),
            Some("farmers")
        );
        assert_eq!(result.state_pops[0].demand_size, Some(10_000.0));
        assert_eq!(result.state_pops[0].workforce, Some(6_000.0));
        assert_eq!(result.state_pops[0].dependents, Some(4_000.0));
        assert_eq!(result.state_pops[0].literate, Some(1_200.0));
        assert_eq!(result.state_pops[0].workplace_id, Some(1));
        assert_eq!(
            result.state_pops[0].culture_id.as_deref(),
            Some("north_german")
        );
        assert!(
            result.state_pops[0]
                .qualifications
                .iter()
                .any(|row| row.profession_id == "academics" && (row.count - 1.5).abs() < 1e-9),
            "index 0 should map to academics"
        );
        assert_eq!(result.buildings[0].employees[0].profession_id, "farmers");
        assert_eq!(result.buildings[0].employees[0].count, 6_000.0);
        assert!(result
            .state_qualifications
            .iter()
            .any(|row| row.state_id == 1
                && row.profession_id == "farmers"
                && row.employed == 6_000.0));
        assert!(!result.state_pops[0].needs.is_empty());
        assert!(!result.state_needs.is_empty());
        assert!(
            result
                .goods
                .iter()
                .any(|good| (good.price - good.base).abs() > crate::ORDER_EPS),
            "realistic saved IO should move at least one price"
        );
        assert!(
            world
                .frozen_buy
                .as_slice()
                .iter()
                .chain(world.frozen_sell.as_slice())
                .all(|quantity| quantity.abs() <= crate::ORDER_EPS),
            "fixture trade route has no export direction"
        );
    }

    #[test]
    #[ignore = "set VIC3_SAVE and VIC3_GAME to run against a real install"]
    fn live_save_reconstructs_non_base_prices() {
        let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE must point at a .v3");
        let game_path = std::env::var("VIC3_GAME").expect("VIC3_GAME must point at the game root");
        let save = vic3_load::load_path(save_path, vic3_load::empty_tokens())
            .expect("live plaintext save");
        let defs = vic3_defs::load_from_path(game_path).expect("live game definitions");
        assert_eq!(
            defs.good_by_index(GoodIdx::from_usize(18)),
            Some("merchant_marine")
        );
        let world = World::from_save(&save, &defs);
        assert!(
            !world.state_trade.is_empty(),
            "post-1.9 save should contain state-attributed trade"
        );
        let (buy, sell) = reconstruct_non_pop_orders(&world, &defs);
        assert!(buy
            .as_slice()
            .iter()
            .chain(sell.as_slice())
            .any(|quantity| quantity.abs() > crate::ORDER_EPS));
        assert!(!world.buildings.is_empty());
        assert!(
            defs.goods.iter().any(|(id, good)| {
                let Some(idx) = defs.index_of(id) else {
                    return false;
                };
                let price =
                    crate::formula::price(good.base_price, buy[idx], sell[idx], defs.price_range);
                (price - good.base_price).abs() > crate::ORDER_EPS
            }),
            "live saved building IO should imply at least one non-base price"
        );
    }
}
