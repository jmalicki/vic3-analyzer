//! Market-price equilibrium for a Victoria 3 save snapshot.
//!
//! # Pipeline
//!
//! `vic3-load` + `vic3-defs` → [`World`] + [`vic3_defs::GameDefs`] →
//! [`equilibrate`] / [`solve`] / [`preview`] / [`alerts`]. Downstream:
//!
//! - [`equilibrate`] returns a compact [`SolveOutcome`] (prices + building
//!   revenues) for planning / hot re-solves.
//! - [`solve`] packages that into a full [`PricesResult`] via [`report`].
//! - `vic3-planning::PlanningState::from_world_with_prices` copies good prices
//!   (and modeled GDP) into the compact planning IR.
//! - `vic3-api` exposes the same JSON to CLI, wasm, Tauri, and MCP/SQL hosts
//!   (`prices`, `what_if`, `preview`, `alerts`, planning/gaps paths).
//! - `vic3-sql` diagnostics (`good_price`, `shortage_analysis`, …) and MCP
//!   `query` read that session’s last solve; they do not re-derive the NLS.
//!
//! Narrative design notes: [`docs/prices.md`](../../../docs/prices.md).
//!
//! # Inner problem
//!
//! Find relative prices `r` (price / base) such that pop consumption at each
//! state’s MAPI-blended **local** prices is consistent with the closed-form
//! market formula. Building IO, employment, wages, and trade volumes are
//! **frozen** except explicit [`WorldDelta`] / what-if edits. Pop demand is
//! **not** frozen — it sits inside the residual.
//!
//! ```text
//! min ‖r − r_formula(orders(r))‖²
//! subject to r ∈ [1 − PRICE_RANGE, 1 + PRICE_RANGE]
//! ```
//!
//! # Why Basin + successive substitution
//!
//! - **Successive substitution** `r ← (1−α)r + α P(c(r))` is cheap and usually
//!   lands near the fixed point; it is the warm start and the polish / fallback
//!   when Basin reports failure or after a bound-clamped TRF finish.
//! - **Basin `Trf`** (trust-region-reflective, dense Vec Jacobian) finishes the
//!   bound-constrained NLS. Wasm-safe (no BLAS). When [`SolveOpts::warm_rel`]
//!   matches the goods vector length, Basin starts from that vector and skips
//!   successive substitution (CLI `mutate` / wasm apply-delta do this).
//!
//! Residual and [`LIMITATIONS`] are always part of the answer (I5).
//!
//! # Frozen labor / trade (planning implication)
//!
//! Plans that only change building levels or PMs re-equilibrate **prices and
//! pop baskets**, not hire/fire, wages, or trade-center volumes. That keeps
//! the inner problem convex-ish and fast, and matches the architecture freeze
//! list: employment and trade are outer-loop / later-phase work.

mod alerts;
mod consumption;
mod formula;
mod label;
mod optimize;
mod qualification_advice;
mod report;
mod result;
mod shop_cache;
mod solve;
mod world;

use vic3_defs::GameDefs;

pub use vic3_defs::BuildingTypeIdx;

pub use alerts::{
    alerts, alerts_with, goods_shortage_alerts, Alert, AlertKind, AlertsOptions, AlertsResult,
    Evidence, Mitigation, MitigationAction,
};
pub use consumption::consumption;
pub use formula::{
    effective_mapi, local_price, market_access, market_ratio, price, BASE_MAPI, ORDER_EPS,
};
pub use label::{pretty_id, script_label};
pub use optimize::{
    optimize_pms, OptimizeAxis, OptimizeChange, OptimizeDelta, OptimizePmsOpts,
    OptimizePricesSummary, OptimizeResult, MAX_PM_TRIALS, PM_SEARCH_HEURISTIC,
    PM_TECH_GATING_INCOMPLETE,
};
pub use qualification_advice::{BuildingStaffing, ProfessionGap};
pub use report::report;
pub use result::{
    BuildingEconomics, BuildingGroupInfo, BuildingRevenue, BuildingTypeInfo, CountryInfo,
    ExtraLevelsDelta, GoodFlow, GoodPrice, MarketInputs, PopNeedBasket, PricesResult,
    ProductionMethodDelta, ProfessionCount, SolveOpts, SolveOutcome, SolveStatus, StateGood,
    StateInfo, StateNeed, StatePop, StateQualification, SubsidizeDelta, WhatIfOpts, WorldDelta,
};
pub use shop_cache::{BuildingIo, ShopCache, StateShop};
pub use solve::{equilibrate, equilibrate_cached, solve, what_if};
pub use world::{
    reconstruct_non_pop_orders, ConstructionQueueKind, Intern, World, WorldBuilding,
    WorldConstruction, WorldCountry, WorldPop, WorldState, WorldStatePop, WorldStateTrade,
    POP_SCALE,
};

/// Solver caveats copied into every [`PricesResult::limitations`] (CLI / UI / SQL).
///
/// Strings must stay aligned with [`docs/prices.md`](../../../docs/prices.md)
/// “Limitations”. Tests assert the exact wording.
///
/// 1. Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.
/// 2. Prices are clamped to ±PRICE_RANGE; the clamp is part of the model.
/// 3. Employment, wages, and trade volumes are frozen except explicit what-if deltas.
/// 4. Pops shop at each state's MAPI-blended local prices; those orders are access-scaled into one whole-save market. Extra MAPI modifiers and overseas constraints are not modeled.
/// 5. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.
pub const LIMITATIONS: &[&str] = &[
    "Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.",
    "Prices are clamped to ±PRICE_RANGE; the clamp is part of the model.",
    "Employment, wages, and trade volumes are frozen except explicit what-if deltas.",
    "Pops shop at each state's MAPI-blended local prices; state orders are infrastructure-access-scaled into one whole-save market; missing access defaults to 100%, and extra MAPI modifiers and overseas convoy constraints are not modeled.",
    "The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.",
];

/// Appended to [`PricesResult::limitations`] when a [`WorldDelta`] asks to subsidize.
pub const SUBSIDY_NOT_MODELED: &str = "Subsidies are not modeled; subsidy toggles were ignored.";

/// Clone `world` and apply extra levels, then production methods.
///
/// Subsidy entries are accepted and ignored (no IR subsidy flag). Unknown
/// building ids are no-ops. Does not mutate `world`.
///
/// # Arguments
///
/// * `world` — baseline snapshot; left unchanged.
/// * `delta` — see [`WorldDelta`]: PM swaps clear that building’s saved IO;
///   extra levels keep and scale saved IO; `subsidize` is ignored here.
///
/// # Returns
///
/// A new [`World`]. Pair with [`solve`] or use [`preview`] to re-solve in one call.
pub fn apply_delta(world: &World, delta: &WorldDelta) -> World {
    let mut next = world.clone();
    for extra in &delta.extra_levels {
        if let Some(id) = extra.building_id {
            if let Some(building) = next.buildings.iter_mut().find(|b| b.id == id) {
                building.add_extra_levels(extra.extra_levels);
            }
        } else if let Some(kind) = extra.building_type_id {
            for building in &mut next.buildings {
                if building.type_id == kind {
                    building.add_extra_levels(extra.extra_levels);
                }
            }
        }
    }
    for pm in &delta.production_methods {
        if let Some(building) = next.buildings.iter_mut().find(|b| b.id == pm.building_id) {
            *building = building.with_methods(pm.methods.clone());
        }
    }
    next
}

/// Apply [`WorldDelta`] to a clone of `world` and re-solve. `world` is unchanged.
///
/// Prefer passing the baseline [`PricesResult::relative`] as
/// [`SolveOpts::warm_rel`] so Basin skips successive substitution (what
/// `vic3-api` mutate / apply-delta paths do).
///
/// # Arguments
///
/// * `world` / `defs` — same contract as [`solve`].
/// * `delta` — applied via [`apply_delta`] before the solve.
/// * `opts` — residual / iteration / warm-start controls.
///
/// # Returns
///
/// A full [`PricesResult`]. If `delta.subsidize` is non-empty,
/// [`SUBSIDY_NOT_MODELED`] is appended to `limitations` (goods unchanged vs a
/// subsidy-free delta).
///
/// Never fails with a Rust `Err`; insolvable markets surface as
/// [`SolveStatus::Failed`] / [`SolveStatus::MaxIters`] plus residual.
pub fn preview(
    world: &World,
    defs: &GameDefs,
    delta: &WorldDelta,
    opts: SolveOpts,
) -> PricesResult {
    let next = apply_delta(world, delta);
    let mut result = solve(&next, defs, opts);
    if !delta.subsidize.is_empty() {
        result.limitations.push(SUBSIDY_NOT_MODELED.to_string());
    }
    result
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    use proptest::prelude::*;
    use vic3_defs::{
        BuyPackage, GameDefs, Good, GoodIdx, GoodsVec, NeedEntry, NeedIdx, NeedsVec, PopNeed,
        ProductionMethod,
    };

    fn heating_defs() -> GameDefs {
        let heat = NeedIdx::from_usize(0);
        let mut defs = GameDefs {
            price_range: 0.75,
            goods_order: vec!["grain".into(), "wood".into(), "coal".into()],
            needs_order: vec!["popneed_heating".into()],
            pop_needs: vec![PopNeed {
                id: "popneed_heating".into(),
                default_good: Some(GoodIdx::from_usize(1)),
                entries: vec![
                    NeedEntry {
                        good: GoodIdx::from_usize(1),
                        weight: 1.0,
                        min_supply_share: 0.0,
                        max_supply_share: 0.5,
                    },
                    NeedEntry {
                        good: GoodIdx::from_usize(2),
                        weight: 2.0,
                        min_supply_share: 0.1,
                        max_supply_share: 1.0,
                    },
                ],
            }],
            ..GameDefs::default()
        };
        defs.goods.insert(
            "grain".into(),
            Good {
                id: "grain".into(),
                base_price: 20.0,
                traded_quantity: 12.0,
                texture: None,
            },
        );
        defs.goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
                traded_quantity: 10.0,
                texture: None,
            },
        );
        defs.goods.insert(
            "coal".into(),
            Good {
                id: "coal".into(),
                base_price: 30.0,
                traded_quantity: 6.0,
                texture: None,
            },
        );
        let mut needs1 = NeedsVec::zeros(1);
        needs1[heat] = 15.0;
        defs.buy_packages.insert(
            1,
            BuyPackage {
                wealth: 1,
                political_strength: 0.03,
                needs: needs1,
            },
        );
        let mut needs2 = NeedsVec::zeros(1);
        needs2[heat] = 17.0;
        defs.buy_packages.insert(
            2,
            BuyPackage {
                wealth: 2,
                political_strength: 0.04,
                needs: needs2,
            },
        );
        defs.production_methods.insert(
            "pm_simple_forestry".into(),
            ProductionMethod {
                id: "pm_simple_forestry".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodIdx::from_usize(1), 30.0)],
                ..Default::default()
            },
        );
        defs.rebuild_package_ladder();
        defs
    }

    /// Two goods, two singleton needs — buy can equal sell at base prices.
    fn two_good_defs() -> GameDefs {
        let staple = NeedIdx::from_usize(0);
        let heat = NeedIdx::from_usize(1);
        let mut defs = GameDefs {
            price_range: 0.75,
            goods_order: vec!["grain".into(), "wood".into()],
            needs_order: vec!["popneed_staple".into(), "popneed_heating".into()],
            pop_needs: vec![
                PopNeed {
                    id: "popneed_staple".into(),
                    default_good: Some(GoodIdx::from_usize(0)),
                    entries: vec![NeedEntry {
                        good: GoodIdx::from_usize(0),
                        weight: 1.0,
                        min_supply_share: 0.0,
                        max_supply_share: 1.0,
                    }],
                },
                PopNeed {
                    id: "popneed_heating".into(),
                    default_good: Some(GoodIdx::from_usize(1)),
                    entries: vec![NeedEntry {
                        good: GoodIdx::from_usize(1),
                        weight: 1.0,
                        min_supply_share: 0.0,
                        max_supply_share: 1.0,
                    }],
                },
            ],
            ..GameDefs::default()
        };
        defs.goods.insert(
            "grain".into(),
            Good {
                id: "grain".into(),
                base_price: 20.0,
                traded_quantity: 12.0,
                texture: None,
            },
        );
        defs.goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
                traded_quantity: 10.0,
                texture: None,
            },
        );
        let mut needs1 = NeedsVec::zeros(2);
        needs1[staple] = 20.0;
        needs1[heat] = 20.0;
        defs.buy_packages.insert(
            1,
            BuyPackage {
                wealth: 1,
                political_strength: 0.03,
                needs: needs1,
            },
        );
        let mut needs2 = NeedsVec::zeros(2);
        needs2[staple] = 22.0;
        needs2[heat] = 22.0;
        defs.buy_packages.insert(
            2,
            BuyPackage {
                wealth: 2,
                political_strength: 0.04,
                needs: needs2,
            },
        );
        defs.production_methods.insert(
            "pm_simple_forestry".into(),
            ProductionMethod {
                id: "pm_simple_forestry".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodIdx::from_usize(1), 30.0)],
                ..Default::default()
            },
        );
        defs.rebuild_package_ladder();
        defs.ensure_building_type("logging_camp");
        defs
    }

    fn two_pops() -> Vec<WorldPop> {
        vec![
            WorldPop {
                state: None,
                size: POP_SCALE,
                wealth: 1,
                wages: 0.0,
                culture: None,
                profession: None,
            },
            WorldPop {
                state: None,
                size: POP_SCALE,
                wealth: 1,
                wages: 0.0,
                culture: None,
                profession: None,
            },
        ]
    }

    /// Two goods (grain/wood), two pops; frozen sell matched to pop buy at base.
    fn balanced_world(defs: &GameDefs) -> World {
        let pops = two_pops();
        let prices: GoodsVec = defs
            .goods_order
            .iter()
            .map(|id| defs.base_price(id).unwrap_or(0.0))
            .collect();
        let sell = consumption(
            &pops,
            &prices,
            &prices,
            defs,
            &GoodsVec::zeros(defs.goods_order.len()),
        );
        World {
            pops,
            buildings: vec![WorldBuilding {
                id: 1,
                state: None,
                type_id: defs.building_index_of("logging_camp").unwrap(),
                level: 0.0,
                staffing: 1.0,
                production_methods: vec!["pm_simple_forestry".into()],
                saved_inputs: Default::default(),
                saved_outputs: Default::default(),
            }],
            frozen_sell: sell,
            ..World::default()
        }
    }

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn fixture_forestry_orders_move_wood_below_base_price() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-defs/tests/fixtures");
        let mut defs = vic3_defs::load_from_path(root).expect("defs fixture");
        // Fixture buildings file has no logging camp; PM recipes are enough for IO.
        let logging = defs.ensure_building_type("building_logging_camp");
        let world = World {
            buildings: vec![WorldBuilding {
                id: 1,
                state: None,
                type_id: logging,
                level: 2.0,
                staffing: 2.0,
                production_methods: vec!["pm_simple_forestry".into()],
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            }],
            ..World::default()
        };

        let result = solve(&world, &defs, SolveOpts::default());
        assert!(result.inputs.goods_with_orders > 0);
        let wood = result
            .goods
            .iter()
            .find(|good| good.id == "wood")
            .expect("wood price");
        assert_eq!(wood.sell, 60.0);
        assert_ne!(wood.price, wood.base);
        assert!(
            wood.price < wood.base,
            "forestry oversupply should lower wood"
        );
    }

    #[test]
    fn limitations_nonempty_and_matches_const() {
        assert!(!LIMITATIONS.is_empty());
        assert_eq!(LIMITATIONS.len(), 5);
        assert_eq!(
            LIMITATIONS[0],
            "Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve."
        );
        assert_eq!(
            LIMITATIONS[1],
            "Prices are clamped to ±PRICE_RANGE; the clamp is part of the model."
        );
        assert_eq!(
            LIMITATIONS[2],
            "Employment, wages, and trade volumes are frozen except explicit what-if deltas."
        );
        assert_eq!(
            LIMITATIONS[3],
            "Pops shop at each state's MAPI-blended local prices; state orders are infrastructure-access-scaled into one whole-save market; missing access defaults to 100%, and extra MAPI modifiers and overseas convoy constraints are not modeled."
        );
        assert_eq!(
            LIMITATIONS[4],
            "The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point."
        );
        let result = solve(
            &World::default(),
            &GameDefs::default(),
            SolveOpts::default(),
        );
        assert_eq!(
            result.limitations,
            LIMITATIONS
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_good_two_pop_known_equilibrium() {
        let defs = two_good_defs();
        let world = balanced_world(&defs);
        let result = solve(&world, &defs, SolveOpts::default());
        assert_eq!(result.status, SolveStatus::Converged);
        assert!(result.residual < SolveOpts::default().residual_eps);
        assert_eq!(result.relative.len(), result.goods.len());
        for (row, &rel) in result.goods.iter().zip(&result.relative) {
            assert!((row.price - row.base * rel).abs() < 1e-9);
        }
        let warm = solve(
            &world,
            &defs,
            SolveOpts {
                warm_rel: Some(result.relative.clone()),
                ..SolveOpts::default()
            },
        );
        for (left, right) in result.goods.iter().zip(&warm.goods) {
            assert!(
                (left.price - right.price).abs() < 1e-4,
                "{} cold {} vs warm {}",
                left.id,
                left.price,
                right.price
            );
        }
        for row in &result.goods {
            let scale = 1e-6_f64.max(1e-6 * row.base.abs());
            assert!(
                (row.price - row.base).abs() < scale * 50.0,
                "{} price {} vs base {}",
                row.id,
                row.price,
                row.base
            );
            assert!(
                (row.buy - row.sell).abs() < 1e-4 * (1.0 + row.buy.abs()),
                "{} buy {} vs sell {}",
                row.id,
                row.buy,
                row.sell
            );
        }
    }

    #[test]
    fn state_and_building_detail_uses_pm_io_and_local_prices() {
        let mut defs = heating_defs();
        defs.labels.insert("wood".into(), "Wood".into());
        defs.production_methods.insert(
            "pm_goofy_factory".into(),
            ProductionMethod {
                id: "pm_goofy_factory".into(),
                inputs: vec![(GoodIdx::from_usize(2), 2.0)],
                outputs: vec![(GoodIdx::from_usize(1), 3.0)],
                ..Default::default()
            },
        );
        let world = World {
            states: vec![WorldState {
                id: 7,
                region: Some("STATE_TESTOPIA".into()),
                country: Some(1),
                market: Some(2),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 9,
                state: Some(7),
                type_id: defs.ensure_building_type("building_goofy_factory"),
                level: 2.0,
                staffing: 1.0,
                production_methods: vec!["pm_goofy_factory".into()],
                saved_inputs: Default::default(),
                saved_outputs: Default::default(),
            }],
            ..World::default()
        };
        let result = solve(&world, &defs, SolveOpts::default());
        assert_eq!(result.scope, "whole_save_synthetic");
        assert_eq!(
            result.states[0].region_id.as_deref(),
            Some("STATE_TESTOPIA")
        );
        assert_eq!(
            result
                .goods
                .iter()
                .find(|good| good.id == "wood")
                .unwrap()
                .name
                .as_deref(),
            Some("Wood")
        );
        let building = &result.buildings[0];
        assert_eq!(building.state_id, Some(7));
        assert_eq!(building.inputs[0].quantity, 2.0);
        assert_eq!(building.outputs[0].quantity, 3.0);
        assert_eq!(building.profit, building.revenue - building.cost);
        assert!(result
            .state_goods
            .iter()
            .any(|row| row.state_id == 7 && row.good_id == "wood" && row.sell == 3.0));
    }

    #[test]
    fn state_prices_follow_local_attributed_orders() {
        let mut defs = heating_defs();
        defs.production_methods.insert(
            "pm_wood_buyer".into(),
            ProductionMethod {
                id: "pm_wood_buyer".into(),
                inputs: vec![(GoodIdx::from_usize(1), 10.0)],
                outputs: Vec::new(),
                ..Default::default()
            },
        );
        defs.production_methods.insert(
            "pm_wood_seller".into(),
            ProductionMethod {
                id: "pm_wood_seller".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodIdx::from_usize(1), 10.0)],
                ..Default::default()
            },
        );
        let state = |id| WorldState {
            id,
            country: Some(1),
            market: Some(1),
            infrastructure: (id == 1).then_some(45.0),
            infrastructure_usage: (id == 1).then_some(90.0),
            ..WorldState::default()
        };
        let building = |defs: &mut GameDefs, id, state, method: &str| WorldBuilding {
            id,
            state: Some(state),
            type_id: defs.ensure_building_type(&format!("building_{method}")),
            level: 1.0,
            staffing: 1.0,
            production_methods: vec![method.into()],
            saved_inputs: Default::default(),
            saved_outputs: Default::default(),
        };
        let world = World {
            states: vec![state(1), state(2)],
            buildings: vec![
                building(&mut defs, 1, 1, "pm_wood_buyer"),
                building(&mut defs, 2, 2, "pm_wood_seller"),
            ],
            ..World::default()
        };

        let result = solve(&world, &defs, SolveOpts::default());
        let market_wood = result
            .goods
            .iter()
            .find(|row| row.id == "wood")
            .expect("market wood row");
        assert_eq!(market_wood.buy, 5.0);
        assert_eq!(market_wood.sell, 10.0);
        let wood = |state_id| {
            result
                .state_goods
                .iter()
                .find(|row| row.state_id == state_id && row.good_id == "wood")
                .expect("wood state row")
        };
        assert!(wood(1).price > wood(1).base);
        assert!(wood(2).price < wood(2).base);
        assert_ne!(wood(1).price, wood(2).price);
        assert_eq!(wood(1).market_access, 0.5);
        assert_eq!(wood(1).effective_mapi, 0.375);
        assert_eq!(wood(2).market_access, 1.0);
        assert_eq!(wood(2).effective_mapi, 0.75);
        assert_eq!(
            wood(1).price,
            local_price(
                wood(1).effective_mapi,
                wood(1).market_price,
                wood(1).state_price
            )
        );
    }

    #[test]
    fn wage_pops_shop_at_local_prices_not_world() {
        let mut defs = heating_defs();
        defs.production_methods.insert(
            "pm_wood_buyer".into(),
            ProductionMethod {
                id: "pm_wood_buyer".into(),
                inputs: vec![(GoodIdx::from_usize(1), 10.0)],
                outputs: Vec::new(),
                ..Default::default()
            },
        );
        defs.production_methods.insert(
            "pm_wood_seller".into(),
            ProductionMethod {
                id: "pm_wood_seller".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodIdx::from_usize(1), 10.0)],
                ..Default::default()
            },
        );
        let state = |id| WorldState {
            id,
            country: Some(1),
            market: Some(1),
            infrastructure: (id == 1).then_some(45.0),
            infrastructure_usage: (id == 1).then_some(90.0),
            ..WorldState::default()
        };
        let building = |defs: &mut GameDefs, id, state, method: &str| WorldBuilding {
            id,
            state: Some(state),
            type_id: defs.ensure_building_type(&format!("building_{method}")),
            level: 1.0,
            staffing: 1.0,
            production_methods: vec![method.into()],
            saved_inputs: Default::default(),
            saved_outputs: Default::default(),
        };
        let wage_pop = |state| WorldPop {
            state: Some(state),
            size: POP_SCALE,
            wealth: 1,
            wages: 1.0,
            culture: None,
            profession: None,
        };
        let world = World {
            states: vec![state(1), state(2)],
            buildings: vec![
                building(&mut defs, 1, 1, "pm_wood_buyer"),
                building(&mut defs, 2, 2, "pm_wood_seller"),
            ],
            pops: vec![wage_pop(1), wage_pop(2)],
            ..World::default()
        };

        let result = solve(&world, &defs, SolveOpts::default());
        let heating = |state_id: u32| {
            result
                .state_pops
                .iter()
                .find(|pop| pop.state_id == state_id)
                .and_then(|pop| pop.needs.first())
                .expect("heating basket")
        };
        let poor_access = heating(1);
        let full_access = heating(2);
        assert_ne!(
            poor_access.package_value, full_access.package_value,
            "local cost of living must move wage-pop packages apart"
        );
        assert!(
            full_access.package_value > poor_access.package_value,
            "cheap local wood should raise real income / package size, got poor={} full={}",
            poor_access.package_value,
            full_access.package_value
        );

        let wood_row = |state_id: u32| {
            result
                .state_goods
                .iter()
                .find(|row| row.state_id == state_id && row.good_id == "wood")
                .expect("state wood")
        };
        let pop_wood_1 = wood_row(1).buy - 10.0;
        let pop_wood_2 = wood_row(2).buy;
        assert!(
            (pop_wood_1 - pop_wood_2).abs() > crate::ORDER_EPS,
            "unscaled pop wood buy must differ when local prices differ ({pop_wood_1} vs {pop_wood_2})"
        );

        let buyer = result
            .buildings
            .iter()
            .find(|building| building.state_id == Some(1))
            .expect("buyer");
        let local_wood = result
            .state_goods
            .iter()
            .find(|row| row.state_id == 1 && row.good_id == "wood")
            .expect("local wood");
        assert_eq!(buyer.inputs[0].good_id, "wood");
        assert!(
            (buyer.inputs[0].value / buyer.inputs[0].quantity - local_wood.price).abs() < 1e-9,
            "building IO is valued at the state's local price"
        );
    }

    #[test]
    fn modern_state_trade_is_attributed_and_access_scaled() {
        let defs = heating_defs();
        let wood = defs.index_of("wood").expect("wood index");
        let state = |id, access: f64| WorldState {
            id,
            country: Some(1),
            market: Some(1),
            infrastructure: Some(access * 100.0),
            infrastructure_usage: Some(100.0),
            ..WorldState::default()
        };
        let world = World {
            states: vec![state(1, 0.5), state(2, 1.0)],
            state_trade: vec![
                WorldStateTrade {
                    state: 1,
                    good: wood,
                    quantity: 10.0,
                },
                WorldStateTrade {
                    state: 2,
                    good: wood,
                    quantity: -8.0,
                },
            ],
            ..World::default()
        };

        let result = solve(&world, &defs, SolveOpts::default());
        let market_wood = result
            .goods
            .iter()
            .find(|row| row.id == "wood")
            .expect("market wood row");
        assert_eq!(market_wood.buy, 8.0);
        assert_eq!(market_wood.sell, 5.0);
        let local = |state_id| {
            result
                .state_goods
                .iter()
                .find(|row| row.state_id == state_id && row.good_id == "wood")
                .expect("state wood row")
        };
        assert_eq!(local(1).buy, 0.0);
        assert_eq!(local(1).sell, 10.0);
        assert_eq!(local(2).buy, 8.0);
        assert_eq!(local(2).sell, 0.0);
    }

    #[test]
    fn what_if_extra_levels_changes_price_with_source_world_unchanged() {
        let defs = two_good_defs();
        let world = balanced_world(&defs);
        let staffing_before: Vec<f64> = world.buildings.iter().map(|b| b.staffing).collect();
        let baseline = solve(&world, &defs, SolveOpts::default());
        let bumped = what_if(
            &world,
            &defs,
            &WhatIfOpts {
                building_type_id: defs.building_index_of("logging_camp").unwrap(),
                extra_levels: 1,
            },
            SolveOpts::default(),
        );
        assert_eq!(world.buildings[0].staffing, staffing_before[0]);
        let wood0 = baseline
            .goods
            .iter()
            .find(|g| g.id == "wood")
            .expect("wood");
        let wood1 = bumped.goods.iter().find(|g| g.id == "wood").expect("wood");
        assert!(
            wood1.sell > wood0.sell + 1.0,
            "extra forestry should add wood sell ({} vs {})",
            wood1.sell,
            wood0.sell
        );
        assert!(
            wood1.price <= wood0.price + 1e-9,
            "more wood sell should not raise the wood price ({} vs {})",
            wood1.price,
            wood0.price
        );
        assert_eq!(bumped.limitations, baseline.limitations);
    }

    #[test]
    fn preview_pm_swap_changes_io_and_prices_source_world_unchanged() {
        let mut defs = two_good_defs();
        defs.production_methods.insert(
            "pm_grain".into(),
            ProductionMethod {
                id: "pm_grain".into(),
                inputs: Vec::new(),
                outputs: vec![(GoodIdx::from_usize(0), 30.0)],
                ..Default::default()
            },
        );
        let wood = GoodIdx::from_usize(1);
        let grain = GoodIdx::from_usize(0);
        let world = World {
            pops: two_pops(),
            buildings: vec![WorldBuilding {
                id: 7,
                state: None,
                type_id: defs.ensure_building_type("farm"),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_simple_forestry".into()],
                saved_inputs: Vec::new(),
                saved_outputs: vec![(wood, 99.0)],
            }],
            frozen_sell: GoodsVec::zeros(defs.goods_order.len()),
            ..World::default()
        };
        let baseline = solve(&world, &defs, SolveOpts::default());
        let delta = WorldDelta {
            production_methods: vec![ProductionMethodDelta {
                building_id: 7,
                methods: vec!["pm_grain".into()],
            }],
            ..WorldDelta::default()
        };
        let previewed = preview(&world, &defs, &delta, SolveOpts::default());

        assert_eq!(
            world.buildings[0].production_methods,
            ["pm_simple_forestry"]
        );
        assert_eq!(world.buildings[0].saved_outputs, [(wood, 99.0)]);

        let applied = apply_delta(&world, &delta);
        let (_, sell) = applied.buildings[0].goods_io(&defs);
        assert_eq!(sell[grain], 30.0, "PM recipe × staffed levels");
        assert_eq!(sell[wood], 0.0, "saved wood IO must not survive a PM swap");

        let wood0 = baseline
            .goods
            .iter()
            .find(|g| g.id == "wood")
            .expect("wood");
        let wood1 = previewed
            .goods
            .iter()
            .find(|g| g.id == "wood")
            .expect("wood");
        let grain0 = baseline
            .goods
            .iter()
            .find(|g| g.id == "grain")
            .expect("grain");
        let grain1 = previewed
            .goods
            .iter()
            .find(|g| g.id == "grain")
            .expect("grain");
        assert!(
            wood1.sell < wood0.sell,
            "dropping saved wood IO should lower wood sell ({} vs {})",
            wood1.sell,
            wood0.sell
        );
        assert!(
            grain1.sell > grain0.sell,
            "grain PM should raise grain sell ({} vs {})",
            grain1.sell,
            grain0.sell
        );
    }

    #[test]
    fn preview_extra_levels_on_type_matches_what_if() {
        let defs = two_good_defs();
        let world = balanced_world(&defs);
        let opts = WhatIfOpts {
            building_type_id: defs.building_index_of("logging_camp").unwrap(),
            extra_levels: 1,
        };
        let via_what_if = what_if(&world, &defs, &opts, SolveOpts::default());
        let via_delta = preview(&world, &defs, &WorldDelta::from(opts), SolveOpts::default());
        assert_eq!(via_delta.goods, via_what_if.goods);
        assert_eq!(via_delta.residual, via_what_if.residual);
        assert_eq!(via_delta.status, via_what_if.status);
        assert_eq!(world.buildings[0].level, 0.0, "source world is immutable");
    }

    #[test]
    fn preview_extra_levels_on_trade_center_type() {
        let wood = GoodIdx::from_usize(1);
        let mut defs = GameDefs::default();
        let type_id = defs.ensure_building_type("building_trade_center");
        let world = World {
            buildings: vec![WorldBuilding {
                id: 3,
                state: Some(1),
                type_id,
                level: 2.0,
                staffing: 1.0,
                production_methods: Vec::new(),
                saved_inputs: Vec::new(),
                saved_outputs: vec![(wood, 40.0)],
            }],
            ..World::default()
        };
        let delta = WorldDelta {
            extra_levels: vec![ExtraLevelsDelta {
                building_type_id: Some(type_id),
                building_id: None,
                extra_levels: 2,
            }],
            ..WorldDelta::default()
        };
        let next = apply_delta(&world, &delta);
        assert_eq!(world.buildings[0].level, 2.0, "source world is immutable");
        assert_eq!(next.buildings[0].level, 4.0);
        assert_eq!(next.buildings[0].staffing, 2.0);
        assert_eq!(next.buildings[0].saved_outputs, [(wood, 80.0)]);
    }

    #[test]
    fn preview_subsidy_is_noop_with_limitation() {
        let defs = two_good_defs();
        let world = balanced_world(&defs);
        let baseline = solve(&world, &defs, SolveOpts::default());
        let delta = WorldDelta {
            subsidize: vec![SubsidizeDelta {
                building_id: 1,
                enabled: true,
            }],
            ..WorldDelta::default()
        };
        let previewed = preview(&world, &defs, &delta, SolveOpts::default());
        assert_eq!(apply_delta(&world, &delta), world);
        assert_eq!(previewed.goods, baseline.goods);
        assert!(
            previewed
                .limitations
                .iter()
                .any(|line| line == SUBSIDY_NOT_MODELED),
            "subsidy limitation: {:?}",
            previewed.limitations
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// I5: residual is always reported; `status = converged` ⇒ residual < ε.
        #[test]
        fn i5_converged_implies_residual_below_eps(
            wood_sell in 1.0f64..=200.0,
            coal_sell in 1.0f64..=200.0,
            size_a in 1_000.0f64..=20_000.0,
            size_b in 1_000.0f64..=20_000.0,
        ) {
            let defs = heating_defs();
            let mut frozen_sell = GoodsVec::zeros(defs.goods_order.len());
            frozen_sell[GoodIdx::from_usize(1)] = wood_sell;
            frozen_sell[GoodIdx::from_usize(2)] = coal_sell;
            let world = World {
                pops: vec![
                    WorldPop {
                        state: None,
                        size: size_a,
                        wealth: 1,
                        wages: 0.0,
                        culture: None,
                        profession: None,
                    },
                    WorldPop {
                        state: None,
                        size: size_b,
                        wealth: 1,
                        wages: 0.0,
                        culture: None,
                        profession: None,
                    },
                ],
                frozen_sell,
                ..World::default()
            };
            let opts = SolveOpts::default();
            let result = solve(&world, &defs, opts.clone());
            prop_assert!(result.residual.is_finite());
            prop_assert!(result.residual >= 0.0);
            if result.status == SolveStatus::Converged {
                prop_assert!(result.residual < opts.residual_eps);
            }
            prop_assert_eq!(result.limitations.len(), LIMITATIONS.len());
        }

        /// Warm-starting from a previous solve's relative vector matches a cold start.
        #[test]
        fn warm_start_matches_cold_start_prices(
            wood_sell in 1.0f64..=200.0,
            coal_sell in 1.0f64..=200.0,
            size_a in 1_000.0f64..=20_000.0,
            size_b in 1_000.0f64..=20_000.0,
        ) {
            let defs = heating_defs();
            let mut frozen_sell = GoodsVec::zeros(defs.goods_order.len());
            frozen_sell[GoodIdx::from_usize(1)] = wood_sell;
            frozen_sell[GoodIdx::from_usize(2)] = coal_sell;
            let world = World {
                pops: vec![
                    WorldPop {
                        state: None,
                        size: size_a,
                        wealth: 1,
                        wages: 0.0,
                        culture: None,
                        profession: None,
                    },
                    WorldPop {
                        state: None,
                        size: size_b,
                        wealth: 1,
                        wages: 0.0,
                        culture: None,
                        profession: None,
                    },
                ],
                frozen_sell,
                ..World::default()
            };
            let cold = solve(&world, &defs, SolveOpts::default());
            prop_assert!(!cold.relative.is_empty());
            prop_assert_eq!(cold.relative.len(), cold.goods.len());
            let warm = solve(
                &world,
                &defs,
                SolveOpts {
                    warm_rel: Some(cold.relative.clone()),
                    ..SolveOpts::default()
                },
            );
            prop_assert_eq!(cold.goods.len(), warm.goods.len());
            let tol = 1e-4_f64.max(SolveOpts::default().residual_eps);
            for (left, right) in cold.goods.iter().zip(&warm.goods) {
                prop_assert!(
                    (left.price - right.price).abs() < tol,
                    "{} cold {} vs warm {}",
                    left.id,
                    left.price,
                    right.price
                );
            }
            let mismatch = solve(
                &world,
                &defs,
                SolveOpts {
                    warm_rel: Some(vec![1.0]),
                    ..SolveOpts::default()
                },
            );
            prop_assert_eq!(mismatch.goods.len(), cold.goods.len());
            for (left, right) in cold.goods.iter().zip(&mismatch.goods) {
                prop_assert!(
                    (left.price - right.price).abs() < tol,
                    "length-mismatch warm_rel should cold-start: {} {} vs {}",
                    left.id,
                    left.price,
                    right.price
                );
            }
        }
    }
}
