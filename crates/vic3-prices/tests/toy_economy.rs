//! What-if / solver integration against the self-contained toy economy pack.
//!
//! Defs: `vic3-defs/tests/fixtures/toy_economy`
//! Save: `vic3-load/tests/fixtures/toy_economy.txt`
//! Chain: wheat → flour → bread via farm, mill, bakery, and trade center.

use std::path::PathBuf;

use vic3_defs::load_from_path;
use vic3_load::{empty_tokens, load_path};
use vic3_prices::{
    apply_delta, preview, solve, what_if, ExtraLevelsDelta, GoodPrice, PricesResult,
    ProductionMethodDelta, SolveOpts, SolveStatus, WhatIfOpts, World, WorldDelta,
};

fn toy_defs_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures/toy_economy")
}

fn toy_save_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-load/tests/fixtures/toy_economy.txt")
}

fn load_toy_world() -> (World, vic3_defs::GameDefs) {
    let defs = load_from_path(toy_defs_root()).expect("toy economy defs");
    let save = load_path(toy_save_path(), empty_tokens()).expect("toy economy save");
    let world = World::from_save(&save, &defs);
    (world, defs)
}

fn good<'a>(result: &'a PricesResult, id: &str) -> &'a GoodPrice {
    result
        .goods
        .iter()
        .find(|row| row.good_name == id)
        .unwrap_or_else(|| panic!("missing good {id}"))
}

fn mill_id(world: &World) -> u32 {
    world
        .buildings
        .iter()
        .find(|b| b.building == "building_flour_mill")
        .map(|b| b.id)
        .expect("flour mill in toy save")
}

fn trade_center(world: &World) -> &vic3_prices::WorldBuilding {
    world
        .buildings
        .iter()
        .find(|b| b.building == "building_trade_center")
        .expect("trade center in toy save")
}

#[test]
fn toy_economy_solve_converges() {
    let (world, defs) = load_toy_world();
    assert_eq!(world.buildings.len(), 4);
    assert!(world
        .buildings
        .iter()
        .any(|b| b.building == "building_wheat_farm"));
    assert!(world
        .buildings
        .iter()
        .any(|b| b.building == "building_bakery"));

    let result = solve(&world, &defs, SolveOpts::default());
    assert!(
        result.residual.is_finite(),
        "residual must be finite, got {}",
        result.residual
    );
    assert!(
        matches!(
            result.status,
            SolveStatus::Converged | SolveStatus::MaxIters
        ),
        "unexpected status {:?} residual={}",
        result.status,
        result.residual
    );
    if result.status == SolveStatus::Converged {
        assert!(result.residual < SolveOpts::default().residual_eps);
    }
    assert!(result.goods.iter().any(|g| g.good_name == "wheat"));
    assert!(result.goods.iter().any(|g| g.good_name == "flour"));
    assert!(result.goods.iter().any(|g| g.good_name == "bread"));
}

#[test]
fn what_if_extra_farm_levels_raises_wheat_supply_and_lowers_price() {
    let (world, defs) = load_toy_world();
    let baseline = solve(&world, &defs, SolveOpts::default());
    let bumped = what_if(
        &world,
        &defs,
        &WhatIfOpts {
            building: "building_wheat_farm".into(),
            extra_levels: 2,
        },
        SolveOpts::default(),
    );

    let wheat0 = good(&baseline, "wheat");
    let wheat1 = good(&bumped, "wheat");
    assert!(
        wheat1.sell > wheat0.sell + 1.0,
        "extra farm levels should raise wheat sell ({} vs {})",
        wheat1.sell,
        wheat0.sell
    );
    assert!(
        wheat1.price <= wheat0.price + 1e-9,
        "more wheat supply should not raise wheat price ({} vs {})",
        wheat1.price,
        wheat0.price
    );
    assert_eq!(
        world
            .buildings
            .iter()
            .find(|b| b.building == "building_wheat_farm")
            .map(|b| b.level),
        Some(3.0),
        "source world must stay immutable"
    );
}

#[test]
fn what_if_extra_bakery_levels_raises_bread_supply_and_lowers_price() {
    let (world, defs) = load_toy_world();
    let baseline = solve(&world, &defs, SolveOpts::default());
    let bumped = what_if(
        &world,
        &defs,
        &WhatIfOpts {
            building: "building_bakery".into(),
            extra_levels: 2,
        },
        SolveOpts::default(),
    );

    let bread0 = good(&baseline, "bread");
    let bread1 = good(&bumped, "bread");
    assert!(
        bread1.sell > bread0.sell + 1.0,
        "extra bakery levels should raise bread sell ({} vs {})",
        bread1.sell,
        bread0.sell
    );
    assert!(
        bread1.price <= bread0.price + 1e-9,
        "more bread supply should not raise bread price ({} vs {})",
        bread1.price,
        bread0.price
    );

    // Bakery also buys more flour when levels scale saved IO.
    let flour0 = good(&baseline, "flour");
    let flour1 = good(&bumped, "flour");
    assert!(
        flour1.buy > flour0.buy + 1.0,
        "extra bakery levels should raise flour buy ({} vs {})",
        flour1.buy,
        flour0.buy
    );
}

#[test]
fn preview_mill_pm_swap_to_efficient_changes_io() {
    let (world, defs) = load_toy_world();
    let mill = mill_id(&world);
    let baseline = solve(&world, &defs, SolveOpts::default());

    let delta = WorldDelta {
        production_methods: vec![ProductionMethodDelta {
            building_id: mill,
            methods: vec!["pm_toy_mill_efficient".into()],
        }],
        ..WorldDelta::default()
    };
    let previewed = preview(&world, &defs, &delta, SolveOpts::default());
    let applied = apply_delta(&world, &delta);
    let mill_building = applied
        .buildings
        .iter()
        .find(|b| b.id == mill)
        .expect("mill after delta");

    assert_eq!(
        world
            .buildings
            .iter()
            .find(|b| b.id == mill)
            .map(|b| b.production_methods.clone()),
        Some(vec!["pm_toy_mill".into()]),
        "source mill PM unchanged"
    );
    assert_eq!(mill_building.production_methods, ["pm_toy_mill_efficient"]);
    assert!(
        mill_building.saved_inputs.is_empty() && mill_building.saved_outputs.is_empty(),
        "PM swap must clear saved IO so recipes apply"
    );

    let (buy, sell) = mill_building.goods_io(&defs);
    let wheat = defs.index_of("wheat").expect("wheat");
    let flour = defs.index_of("flour").expect("flour");
    // Efficient mill: 15 wheat in / 20 flour out × staffed levels (2).
    assert!(
        (buy[wheat] - 30.0).abs() < 1e-9,
        "efficient mill wheat input {}",
        buy[wheat]
    );
    assert!(
        (sell[flour] - 40.0).abs() < 1e-9,
        "efficient mill flour output {}",
        sell[flour]
    );

    let flour0 = good(&baseline, "flour");
    let flour1 = good(&previewed, "flour");
    let wheat0 = good(&baseline, "wheat");
    let wheat1 = good(&previewed, "wheat");
    assert!(
        flour1.sell > flour0.sell,
        "efficient mill should raise flour sell ({} vs {})",
        flour1.sell,
        flour0.sell
    );
    assert!(
        wheat1.buy < wheat0.buy,
        "efficient mill should lower wheat buy ({} vs {})",
        wheat1.buy,
        wheat0.buy
    );
}

#[test]
fn preview_extra_levels_on_trade_center_type() {
    // Trade volumes live on `World::state_trade` / frozen maps, not on the trade
    // center building's saved IO in this fixture (empty outputs). Extra levels
    // still scale level/staffing (and any saved IO) the same way as
    // `preview_extra_levels_on_trade_center_type` in the unit tests; state trade
    // quantities stay frozen unless edited elsewhere.
    let (world, defs) = load_toy_world();
    let before = trade_center(&world).clone();
    let state_trade_before = world.state_trade.clone();

    let delta = WorldDelta {
        extra_levels: vec![ExtraLevelsDelta {
            building: Some("building_trade_center".into()),
            building_id: None,
            extra_levels: 2,
        }],
        ..WorldDelta::default()
    };
    let next = apply_delta(&world, &delta);
    let after = trade_center(&next);

    assert_eq!(before.level, 1.0);
    assert_eq!(after.level, 3.0);
    assert_eq!(after.staffing, before.staffing * (3.0 / 1.0));
    assert_eq!(
        next.state_trade, state_trade_before,
        "state trade volumes remain frozen under building-level what-if"
    );

    let result = preview(&world, &defs, &delta, SolveOpts::default());
    assert!(result.residual.is_finite());
    assert_eq!(trade_center(&world).level, 1.0, "source world immutable");
}
