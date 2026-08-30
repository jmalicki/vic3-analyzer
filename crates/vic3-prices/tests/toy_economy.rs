//! What-if / solver integration against the self-contained toy economy pack.
//!
//! Defs: `vic3-defs/tests/fixtures/toy_economy`
//! Save: `vic3-load/tests/fixtures/toy_economy.txt`
//! Chain: wheat → flour → bread via farm, mill, bakery, and trade center.
//!
//! Each solve / what-if / preview test runs against every entry in
//! [`test_strategies`] so new [`SolveStrategy`] variants pick up the same
//! equilibrium checks without duplicating test bodies.

use std::path::PathBuf;

use vic3_defs::load_from_path;
use vic3_load::{empty_tokens, load_path};
use vic3_prices::{
    apply_delta, preview, solve, what_if, ExtraLevelsDelta, GoodPrice, PricesResult,
    ProductionMethodDelta, SolveOpts, SolveStatus, SolveStrategy, WhatIfOpts, World, WorldDelta,
};

/// Solver strategies exercised by the toy-economy integration tests.
///
/// Add [`SolveStrategy::Joint`] here when the coupled solver lands.
fn test_strategies() -> &'static [SolveStrategy] {
    &[SolveStrategy::Nested]
}

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
        .find(|row| row.name == id)
        .unwrap_or_else(|| panic!("missing good {id}"))
}

fn mill_id(world: &World, defs: &vic3_defs::GameDefs) -> u32 {
    let mill = defs
        .building_index_of("building_flour_mill")
        .expect("flour mill def");
    world
        .buildings
        .iter()
        .find(|b| b.building_type_id == mill)
        .map(|b| b.id)
        .expect("flour mill in toy save")
}

fn trade_center<'a>(
    world: &'a World,
    defs: &vic3_defs::GameDefs,
) -> &'a vic3_prices::WorldBuilding {
    let tc = defs
        .building_index_of("building_trade_center")
        .expect("trade center def");
    world
        .buildings
        .iter()
        .find(|b| b.building_type_id == tc)
        .expect("trade center in toy save")
}

#[test]
fn toy_economy_solve_converges() {
    let (world, defs) = load_toy_world();
    assert_eq!(world.buildings.len(), 4);
    let wheat = defs.building_index_of("building_wheat_farm").unwrap();
    let bakery = defs.building_index_of("building_bakery").unwrap();
    assert!(world.buildings.iter().any(|b| b.building_type_id == wheat));
    assert!(world.buildings.iter().any(|b| b.building_type_id == bakery));

    for &strategy in test_strategies() {
        let result = solve(
            &world,
            &defs,
            SolveOpts {
                strategy,
                ..Default::default()
            },
        );
        assert!(
            result.residual.is_finite(),
            "residual must be finite, got {} (strategy: {:?})",
            result.residual,
            strategy
        );
        assert!(
            matches!(
                result.status,
                SolveStatus::Converged | SolveStatus::MaxIters
            ),
            "unexpected status {:?} residual={} (strategy: {:?})",
            result.status,
            result.residual,
            strategy
        );
        if result.status == SolveStatus::Converged {
            assert!(result.residual < SolveOpts::default().residual_eps);
        }
        assert!(result.goods.iter().any(|g| g.name == "wheat"));
        assert!(result.goods.iter().any(|g| g.name == "flour"));
        assert!(result.goods.iter().any(|g| g.name == "bread"));
    }
}

#[test]
fn what_if_extra_farm_levels_raises_wheat_supply_and_lowers_price() {
    let (world, defs) = load_toy_world();
    for &strategy in test_strategies() {
        let opts = SolveOpts {
            strategy,
            ..Default::default()
        };
        let baseline = solve(&world, &defs, opts.clone());
        let bumped = what_if(
            &world,
            &defs,
            &WhatIfOpts {
                building_type_id: defs.building_index_of("building_wheat_farm").unwrap(),
                extra_levels: 2,
            },
            opts,
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
    }
    assert_eq!(
        world
            .buildings
            .iter()
            .find(|b| b.building_type_id == defs.building_index_of("building_wheat_farm").unwrap())
            .map(|b| b.level),
        Some(3.0),
        "source world must stay immutable"
    );
}

#[test]
fn what_if_extra_bakery_levels_raises_bread_supply_and_lowers_price() {
    let (world, defs) = load_toy_world();
    for &strategy in test_strategies() {
        let opts = SolveOpts {
            strategy,
            ..Default::default()
        };
        let baseline = solve(&world, &defs, opts.clone());
        let bumped = what_if(
            &world,
            &defs,
            &WhatIfOpts {
                building_type_id: defs.building_index_of("building_bakery").unwrap(),
                extra_levels: 2,
            },
            opts,
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
}

#[test]
fn preview_mill_pm_swap_to_efficient_changes_io() {
    let (world, defs) = load_toy_world();
    let mill = mill_id(&world, &defs);
    for &strategy in test_strategies() {
        let opts = SolveOpts {
            strategy,
            ..Default::default()
        };
        let baseline = solve(&world, &defs, opts.clone());

        let delta = WorldDelta {
            production_methods: vec![ProductionMethodDelta {
                building_id: mill,
                methods: vec!["pm_toy_mill_efficient".into()],
            }],
            ..WorldDelta::default()
        };
        let previewed = preview(&world, &defs, &delta, opts);
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
}

#[test]
fn preview_extra_levels_on_trade_center_type() {
    // Trade volumes live on `World::state_trade` / frozen maps, not on the trade
    // center building's saved IO in this fixture (empty outputs). Extra levels
    // still scale level/staffing (and any saved IO) the same way as
    // `preview_extra_levels_on_trade_center_type` in the unit tests; state trade
    // quantities stay frozen unless edited elsewhere.
    let (world, defs) = load_toy_world();
    let before = trade_center(&world, &defs).clone();
    let state_trade_before = world.state_trade.clone();

    let delta = WorldDelta {
        extra_levels: vec![ExtraLevelsDelta {
            building_type_id: Some(defs.building_index_of("building_trade_center").unwrap()),
            building_id: None,
            extra_levels: 2,
        }],
        ..WorldDelta::default()
    };
    let next = apply_delta(&world, &delta);
    let after = trade_center(&next, &defs);

    assert_eq!(before.level, 1.0);
    assert_eq!(after.level, 3.0);
    assert_eq!(after.staffing, before.staffing * (3.0 / 1.0));
    assert_eq!(
        next.state_trade, state_trade_before,
        "state trade volumes remain frozen under building-level what-if"
    );

    for &strategy in test_strategies() {
        let result = preview(
            &world,
            &defs,
            &delta,
            SolveOpts {
                strategy,
                ..Default::default()
            },
        );
        assert!(result.residual.is_finite());
        assert_eq!(
            trade_center(&world, &defs).level,
            1.0,
            "source world immutable"
        );
    }
}
