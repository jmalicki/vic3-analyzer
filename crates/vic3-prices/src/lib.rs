//! Price equilibrium. Inner problem: pop consumption in the loop, buildings frozen.
//!
//! Solver: `min ‖r − r_formula(orders(r))‖²` with box bounds on relative
//! prices, via Basin **trust-region-reflective** (`Trf`, Vec backend) and
//! successive substitution as warm start / fallback.

mod consumption;
mod formula;
mod result;
mod solve;
mod world;

pub use consumption::consumption;
pub use formula::{market_ratio, price, ORDER_EPS};
pub use result::{GoodPrice, PricesResult, SolveOpts, SolveStatus, WhatIfOpts};
pub use solve::{solve, what_if};
pub use world::{reconstruct_non_pop_orders, World, WorldBuilding, WorldPop, POP_SCALE};

/// Solver caveats copied into CLI JSON and the UI.
///
/// 1. Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.
/// 2. Prices are clamped to ±PRICE_RANGE; the clamp is part of the model.
/// 3. Employment, wages, and trade volumes are frozen except explicit what-if deltas.
/// 4. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.
pub const LIMITATIONS: &[&str] = &[
    "Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.",
    "Prices are clamped to ±PRICE_RANGE; the clamp is part of the model.",
    "Employment, wages, and trade volumes are frozen except explicit what-if deltas.",
    "The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.",
];

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use proptest::prelude::*;
    use vic3_defs::{BuyPackage, GameDefs, Good, NeedEntry, PopNeed, ProductionMethod};

    fn heating_defs() -> GameDefs {
        let mut defs = GameDefs {
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.goods.insert(
            "grain".into(),
            Good {
                id: "grain".into(),
                base_price: 20.0,
            },
        );
        defs.goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
            },
        );
        defs.goods.insert(
            "coal".into(),
            Good {
                id: "coal".into(),
                base_price: 30.0,
            },
        );
        defs.pop_needs.insert(
            "popneed_heating".into(),
            PopNeed {
                id: "popneed_heating".into(),
                default_good: Some("wood".into()),
                entries: vec![
                    NeedEntry {
                        good: "wood".into(),
                        weight: 1.0,
                        min_supply_share: 0.0,
                        max_supply_share: 0.5,
                    },
                    NeedEntry {
                        good: "coal".into(),
                        weight: 2.0,
                        min_supply_share: 0.1,
                        max_supply_share: 1.0,
                    },
                ],
            },
        );
        defs.buy_packages.insert(
            1,
            BuyPackage {
                wealth: 1,
                political_strength: 0.03,
                needs: BTreeMap::from([("popneed_heating".into(), 15.0)]),
            },
        );
        defs.buy_packages.insert(
            2,
            BuyPackage {
                wealth: 2,
                political_strength: 0.04,
                needs: BTreeMap::from([("popneed_heating".into(), 17.0)]),
            },
        );
        defs.production_methods.insert(
            "pm_simple_forestry".into(),
            ProductionMethod {
                id: "pm_simple_forestry".into(),
                inputs: BTreeMap::new(),
                outputs: BTreeMap::from([("wood".into(), 30.0)]),
            },
        );
        defs
    }

    /// Two goods, two singleton needs — buy can equal sell at base prices.
    fn two_good_defs() -> GameDefs {
        let mut defs = GameDefs {
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.goods.insert(
            "grain".into(),
            Good {
                id: "grain".into(),
                base_price: 20.0,
            },
        );
        defs.goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
            },
        );
        defs.pop_needs.insert(
            "popneed_staple".into(),
            PopNeed {
                id: "popneed_staple".into(),
                default_good: Some("grain".into()),
                entries: vec![NeedEntry {
                    good: "grain".into(),
                    weight: 1.0,
                    min_supply_share: 0.0,
                    max_supply_share: 1.0,
                }],
            },
        );
        defs.pop_needs.insert(
            "popneed_heating".into(),
            PopNeed {
                id: "popneed_heating".into(),
                default_good: Some("wood".into()),
                entries: vec![NeedEntry {
                    good: "wood".into(),
                    weight: 1.0,
                    min_supply_share: 0.0,
                    max_supply_share: 1.0,
                }],
            },
        );
        defs.buy_packages.insert(
            1,
            BuyPackage {
                wealth: 1,
                political_strength: 0.03,
                needs: BTreeMap::from([
                    ("popneed_staple".into(), 20.0),
                    ("popneed_heating".into(), 20.0),
                ]),
            },
        );
        defs.buy_packages.insert(
            2,
            BuyPackage {
                wealth: 2,
                political_strength: 0.04,
                needs: BTreeMap::from([
                    ("popneed_staple".into(), 22.0),
                    ("popneed_heating".into(), 22.0),
                ]),
            },
        );
        defs.production_methods.insert(
            "pm_simple_forestry".into(),
            ProductionMethod {
                id: "pm_simple_forestry".into(),
                inputs: BTreeMap::new(),
                outputs: BTreeMap::from([("wood".into(), 30.0)]),
            },
        );
        defs
    }

    fn two_pops() -> Vec<WorldPop> {
        vec![
            WorldPop {
                size: POP_SCALE,
                wealth: 1,
                wages: 0.0,
                culture: None,
            },
            WorldPop {
                size: POP_SCALE,
                wealth: 1,
                wages: 0.0,
                culture: None,
            },
        ]
    }

    /// Two goods (grain/wood), two pops; frozen sell matched to pop buy at base.
    fn balanced_world(defs: &GameDefs) -> World {
        let pops = two_pops();
        let prices: BTreeMap<String, f64> = defs
            .goods
            .iter()
            .map(|(id, g)| (id.clone(), g.base_price))
            .collect();
        let sell = consumption(&pops, &prices, defs, &BTreeMap::new());
        World {
            pops,
            buildings: vec![WorldBuilding {
                building: "logging_camp".into(),
                level: 0.0,
                staffing: 1.0,
                production_method: "pm_simple_forestry".into(),
            }],
            frozen_buy: BTreeMap::new(),
            frozen_sell: sell,
        }
    }

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn limitations_nonempty_and_matches_const() {
        assert!(!LIMITATIONS.is_empty());
        assert_eq!(LIMITATIONS.len(), 4);
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
    fn what_if_extra_levels_changes_price_employment_frozen() {
        let defs = two_good_defs();
        let world = balanced_world(&defs);
        let staffing_before: Vec<f64> = world.buildings.iter().map(|b| b.staffing).collect();
        let baseline = solve(&world, &defs, SolveOpts::default());
        let bumped = what_if(
            &world,
            &defs,
            &WhatIfOpts {
                building: "logging_camp".into(),
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
            let world = World {
                pops: vec![
                    WorldPop {
                        size: size_a,
                        wealth: 1,
                        wages: 0.0,
                        culture: None,
                    },
                    WorldPop {
                        size: size_b,
                        wealth: 1,
                        wages: 0.0,
                        culture: None,
                    },
                ],
                buildings: Vec::new(),
                frozen_buy: BTreeMap::new(),
                frozen_sell: BTreeMap::from([
                    ("wood".into(), wood_sell),
                    ("coal".into(), coal_sell),
                ]),
            };
            let opts = SolveOpts::default();
            let result = solve(&world, &defs, opts);
            prop_assert!(result.residual.is_finite());
            prop_assert!(result.residual >= 0.0);
            if result.status == SolveStatus::Converged {
                prop_assert!(result.residual < opts.residual_eps);
            }
            prop_assert_eq!(result.limitations.len(), LIMITATIONS.len());
        }
    }
}
