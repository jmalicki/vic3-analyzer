//! Shared minimal EconomyContext / PlanningState builders for unit tests.
//! Prefer these over copying World/GameDefs blobs into every test module.
//! Not a save file — in-memory World + defs only (faster than mock Save load).

#![allow(dead_code)] // fixtures unused until later planner-test PRs migrate call sites

use std::collections::BTreeMap;

use vic3_defs::{BuildingType, GameDefs, Good, GoodIdx, GoodsVec, ProductionMethod};
use vic3_prices::{SolveOpts, World, WorldBuilding, WorldCountry, WorldState};

use crate::construction::BUILDING_CONSTRUCTION_SECTOR;
use crate::sim::{EconomyContext, SimConfig};
use crate::world::{ConstructionQueueKind, PlanningConstruction, PlanningParts, PlanningState};

pub(crate) struct MiniEconomy {
    pub economy: EconomyContext,
    pub config: SimConfig,
    #[allow(dead_code)] // locked fixture API for callers
    pub country: &'static str, // "GER"
    #[allow(dead_code)] // locked fixture API for callers
    pub state_id: u32, // 1
}

/// Logging camp (wood output) + Construction Sector (country_construction_add=5).
/// Government share 1.0 (no laissez-faire law unless caller sets laws on state).
pub(crate) fn logging_and_cs_economy() -> MiniEconomy {
    let wood = GoodIdx::from_usize(0);
    let mut defs = GameDefs {
        goods_order: vec!["wood".into()],
        goods: BTreeMap::from([(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
                traded_quantity: 10.0,
                texture: None,
            },
        )]),
        price_range: 0.75,
        ..GameDefs::default()
    };
    defs.buildings.insert(
        "building_logging_camp".into(),
        BuildingType {
            id: "building_logging_camp".into(),
            group: None,
            city_type: None,
            production_method_groups: vec!["pmg_logging".into()],
            required_construction: Some(30.0),
        },
    );
    defs.buildings.insert(
        BUILDING_CONSTRUCTION_SECTOR.into(),
        BuildingType {
            id: BUILDING_CONSTRUCTION_SECTOR.into(),
            group: None,
            city_type: None,
            production_method_groups: vec!["pmg_base_building_construction_sector".into()],
            required_construction: Some(10.0),
        },
    );
    defs.production_method_groups
        .insert("pmg_logging".into(), vec!["pm_sawmills".into()]);
    defs.production_method_groups.insert(
        "pmg_base_building_construction_sector".into(),
        vec!["pm_iron_frame_buildings".into()],
    );
    defs.production_methods.insert(
        "pm_sawmills".into(),
        ProductionMethod {
            id: "pm_sawmills".into(),
            outputs: vec![(wood, 10.0)],
            ..ProductionMethod::default()
        },
    );
    defs.production_methods.insert(
        "pm_iron_frame_buildings".into(),
        ProductionMethod {
            id: "pm_iron_frame_buildings".into(),
            country_construction_add: Some(5.0),
            ..ProductionMethod::default()
        },
    );
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
        buildings: vec![
            WorldBuilding {
                id: 1,
                state: Some(1),
                building: "building_logging_camp".into(),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_sawmills".into()],
                saved_inputs: Vec::new(),
                saved_outputs: vec![(wood, 10.0)],
            },
            WorldBuilding {
                id: 2,
                state: Some(1),
                building: BUILDING_CONSTRUCTION_SECTOR.into(),
                level: 0.0,
                staffing: 0.0,
                production_methods: vec!["pm_iron_frame_buildings".into()],
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            },
        ],
        frozen_buy: GoodsVec::from_vec(vec![15.0]),
        ..World::default()
    };
    MiniEconomy {
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

/// Options for [`ger_planning_state`].
pub(crate) struct GerStateOpts {
    pub gdp: f64,
    pub construction_points_per_day: f64,
    pub wood_price: f64,
    /// If true, enqueue one CS job with remaining work points (in-flight).
    pub cs_in_flight: Option<f64>, // remaining points; None = no CS queue
    pub laws: Vec<String>,
}

impl Default for GerStateOpts {
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

/// Parent planning state for GER: set gdp, construction_points_per_day, wood price, optional CS already in constructions queue.
pub(crate) fn ger_planning_state(parts: GerStateOpts) -> PlanningState {
    let (queued_building, constructions) = match parts.cs_in_flight {
        Some(remaining) => (
            Some(BUILDING_CONSTRUCTION_SECTOR.into()),
            vec![PlanningConstruction {
                order_id: 1,
                queue: ConstructionQueueKind::Government,
                state_id: Some(1),
                building: BUILDING_CONSTRUCTION_SECTOR.into(),
                remaining: Some(remaining),
            }],
        ),
        None => (None, Vec::new()),
    };
    PlanningState::from_parts(PlanningParts {
        country: "GER".into(),
        gdp: parts.gdp,
        construction_points_per_day: parts.construction_points_per_day,
        good_prices: vec![("wood".into(), parts.wood_price)],
        queued_building,
        constructions,
        laws: parts.laws,
        ..PlanningParts::default()
    })
}
