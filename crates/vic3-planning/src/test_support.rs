//! Shared minimal EconomyContext / PlanningState builders for unit tests.
//! Prefer these over copying World/GameDefs blobs into every test module.
//! Not a save file — in-memory World + defs only (faster than mock Save load).
//!
//! Typical setup (~3 lines):
//! ```ignore
//! let mini = logging_and_cs_economy();
//! let state = ger_state().gdp(1000.0).points(5.0).wood_price(40.0).get();
//! ```
//! Or customize with fluent `MiniEconomy::ger()` (one line per building/good).

#![allow(dead_code)] // fixtures unused until later planner-test PRs migrate call sites

use vic3_defs::{BuildingType, GameDefs, Good, GoodIdx, GoodsVec, ProductionMethod};
use vic3_prices::{SolveOpts, World, WorldBuilding, WorldCountry, WorldState};

use crate::construction::BUILDING_CONSTRUCTION_SECTOR;
use crate::sim::{EconomyContext, SimConfig};
use crate::world::{ConstructionQueueKind, PlanningConstruction, PlanningParts, PlanningState};

/// Default frozen_buy per good (matches the historical wood fixture value).
const DEFAULT_FROZEN_BUY: f64 = 15.0;
/// Default traded_quantity on registered goods (matches historical fixture).
const DEFAULT_TRADED_QUANTITY: f64 = 10.0;

pub(crate) struct MiniEconomy {
    pub economy: EconomyContext,
    pub config: SimConfig,
    #[allow(dead_code)] // locked fixture API for callers
    pub country: &'static str, // "GER"
    #[allow(dead_code)] // locked fixture API for callers
    pub state_id: u32, // 1
}

/// Register a good in `defs.goods_order` / `defs.goods`. Returns its dense index.
fn register_good(defs: &mut GameDefs, id: &str, base_price: f64) -> GoodIdx {
    let idx = GoodIdx::from_usize(defs.goods_order.len());
    defs.goods_order.push(id.into());
    defs.goods.insert(
        id.into(),
        Good {
            id: id.into(),
            base_price,
            traded_quantity: DEFAULT_TRADED_QUANTITY,
            texture: None,
        },
    );
    idx
}

fn auto_pmg_id(building: &str) -> String {
    let stem = building.strip_prefix("building_").unwrap_or(building);
    format!("pmg_{stem}")
}

fn auto_pm_id(building: &str) -> String {
    let stem = building.strip_prefix("building_").unwrap_or(building);
    format!("pm_{stem}")
}

fn next_building_id(world: &World) -> u32 {
    world.buildings.iter().map(|b| b.id).max().unwrap_or(0) + 1
}

/// Building def + auto PMG/PM + world row (level 1, staffing 1, saved_outputs).
fn register_output_building(
    defs: &mut GameDefs,
    world: &mut World,
    state_id: u32,
    building: &str,
    good_id: &str,
    output_qty: f64,
    required_construction: f64,
) {
    let good = defs
        .index_of(good_id)
        .unwrap_or_else(|| panic!("good `{good_id}` must be registered before output_building"));
    let pmg = auto_pmg_id(building);
    let pm = auto_pm_id(building);
    defs.buildings.insert(
        building.into(),
        BuildingType {
            id: building.into(),
            group: None,
            city_type: None,
            production_method_groups: vec![pmg.clone()],
            required_construction: Some(required_construction),
        },
    );
    defs.production_method_groups.insert(pmg, vec![pm.clone()]);
    defs.production_methods.insert(
        pm.clone(),
        ProductionMethod {
            id: pm.clone(),
            outputs: vec![(good, output_qty)],
            ..ProductionMethod::default()
        },
    );
    world.buildings.push(WorldBuilding {
        id: next_building_id(world),
        state: Some(state_id),
        type_id: building.into(),
        level: 1.0,
        staffing: 1.0,
        production_methods: vec![pm],
        saved_inputs: Vec::new(),
        saved_outputs: vec![(good, output_qty)],
    });
}

/// Construction Sector building + `country_construction_add` PM + world row (level 0).
fn register_construction_sector(
    defs: &mut GameDefs,
    world: &mut World,
    state_id: u32,
    construction_add: f64,
    required_construction: f64,
) {
    let pmg = "pmg_base_building_construction_sector";
    let pm = "pm_iron_frame_buildings";
    defs.buildings.insert(
        BUILDING_CONSTRUCTION_SECTOR.into(),
        BuildingType {
            id: BUILDING_CONSTRUCTION_SECTOR.into(),
            group: None,
            city_type: None,
            production_method_groups: vec![pmg.into()],
            required_construction: Some(required_construction),
        },
    );
    defs.production_method_groups
        .insert(pmg.into(), vec![pm.into()]);
    defs.production_methods.insert(
        pm.into(),
        ProductionMethod {
            id: pm.into(),
            country_construction_add: Some(construction_add),
            ..ProductionMethod::default()
        },
    );
    world.buildings.push(WorldBuilding {
        id: next_building_id(world),
        state: Some(state_id),
        type_id: BUILDING_CONSTRUCTION_SECTOR.into(),
        level: 0.0,
        staffing: 0.0,
        production_methods: vec![pm.into()],
        saved_inputs: Vec::new(),
        saved_outputs: Vec::new(),
    });
}

impl MiniEconomy {
    /// Empty GER / state-1 economy with SimConfig matching the historical fixture.
    pub(crate) fn ger() -> Self {
        let defs = GameDefs {
            price_range: 0.75,
            ..GameDefs::default()
        };
        let world = World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "GER".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 1,
                country: Some(1),
                ..WorldState::default()
            }],
            frozen_buy: GoodsVec::from_vec(Vec::new()),
            ..World::default()
        };
        Self {
            economy: EconomyContext::new(world, defs, SolveOpts::default()),
            config: SimConfig {
                construction_days: 30,
                default_construction_cost: 30,
                max_added_levels_per_type: 5,
                ..SimConfig::default()
            },
            country: "GER",
            state_id: 1,
        }
    }

    fn remap(self, f: impl FnOnce(&mut GameDefs, &mut World, u32)) -> Self {
        let Self {
            economy,
            config,
            country,
            state_id,
        } = self;
        let EconomyContext {
            base_world: mut world,
            mut defs,
            solve_opts,
            ..
        } = economy;
        f(&mut defs, &mut world, state_id);
        Self {
            economy: EconomyContext::new(world, defs, solve_opts),
            config,
            country,
            state_id,
        }
    }

    /// Register a good (base price) and extend `frozen_buy` with the default demand.
    pub(crate) fn good(self, id: &str, base_price: f64) -> Self {
        self.remap(|defs, world, _| {
            register_good(defs, id, base_price);
            let mut buy = world.frozen_buy.as_slice().to_vec();
            buy.push(DEFAULT_FROZEN_BUY);
            world.frozen_buy = GoodsVec::from_vec(buy);
        })
    }

    /// Output building: one line registers def + PM wiring + staffed world row.
    pub(crate) fn output_building(
        self,
        building: &str,
        good_id: &str,
        output_qty: f64,
        required_construction: f64,
    ) -> Self {
        self.remap(|defs, world, state_id| {
            register_output_building(
                defs,
                world,
                state_id,
                building,
                good_id,
                output_qty,
                required_construction,
            );
        })
    }

    /// Construction Sector with given `country_construction_add` and build cost.
    pub(crate) fn construction_sector(
        self,
        construction_add: f64,
        required_construction: f64,
    ) -> Self {
        self.remap(|defs, world, state_id| {
            register_construction_sector(
                defs,
                world,
                state_id,
                construction_add,
                required_construction,
            );
        })
    }

    /// Override `required_construction` on an already-registered building.
    pub(crate) fn building_cost(self, building: &str, required_construction: f64) -> Self {
        self.remap(|defs, _world, _| {
            let bt = defs
                .buildings
                .get_mut(building)
                .unwrap_or_else(|| panic!("building `{building}` must exist before building_cost"));
            bt.required_construction = Some(required_construction);
        })
    }
}

/// Logging camp (wood output) + Construction Sector (`country_construction_add=5`).
/// Government share 1.0 (no laissez-faire law unless caller sets laws on state).
pub(crate) fn logging_and_cs_economy() -> MiniEconomy {
    MiniEconomy::ger()
        .good("wood", 20.0)
        .output_building("building_logging_camp", "wood", 10.0, 30.0)
        .construction_sector(5.0, 10.0)
}

/// Fluent GER planning-state builder. Defaults match the historical fixture.
pub(crate) struct GerStateBuilder {
    gdp: f64,
    construction_points_per_day: f64,
    wood_price: f64,
    /// Remaining work points on an in-flight CS job; `None` = no CS queue.
    cs_in_flight: Option<f64>,
    laws: Vec<String>,
}

impl Default for GerStateBuilder {
    fn default() -> Self {
        Self {
            gdp: 1000.0,
            construction_points_per_day: 1.0,
            wood_price: 30.0,
            cs_in_flight: None,
            laws: Vec::new(),
        }
    }
}

/// Start a GER planning-state builder (state id 1).
pub(crate) fn ger_state() -> GerStateBuilder {
    GerStateBuilder::default()
}

impl GerStateBuilder {
    pub(crate) fn gdp(mut self, gdp: f64) -> Self {
        self.gdp = gdp;
        self
    }

    pub(crate) fn points(mut self, construction_points_per_day: f64) -> Self {
        self.construction_points_per_day = construction_points_per_day;
        self
    }

    pub(crate) fn wood_price(mut self, wood_price: f64) -> Self {
        self.wood_price = wood_price;
        self
    }

    pub(crate) fn cs_in_flight(mut self, remaining: f64) -> Self {
        self.cs_in_flight = Some(remaining);
        self
    }

    pub(crate) fn laws(mut self, laws: Vec<String>) -> Self {
        self.laws = laws;
        self
    }

    pub(crate) fn get(self) -> PlanningState {
        let (queued_building, constructions) = match self.cs_in_flight {
            Some(remaining) => (
                Some(BUILDING_CONSTRUCTION_SECTOR.into()),
                vec![PlanningConstruction {
                    order_id: 1,
                    queue: ConstructionQueueKind::Government,
                    state_id: Some(1),
                    type_id: BUILDING_CONSTRUCTION_SECTOR.into(),
                    remaining: Some(remaining),
                }],
            ),
            None => (None, Vec::new()),
        };
        PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            gdp: self.gdp,
            construction_points_per_day: self.construction_points_per_day,
            good_prices: vec![("wood".into(), self.wood_price)],
            queued_building,
            constructions,
            laws: self.laws,
            ..PlanningParts::default()
        })
    }
}
