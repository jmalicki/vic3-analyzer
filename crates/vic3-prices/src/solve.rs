//! Bound-constrained NLS ([`basin::Trf`]) plus successive-substitution warm start.
//!
//! # Algorithm (same [`equilibrate`] / [`solve`] API)
//!
//! 1. Build access-scaled frozen non-pop orders and state shops (pops at local
//!    prices; buildings + post-1.9 trade frozen).
//! 2. If a valid `warm_rel` is provided, use it as the starting point. Otherwise,
//!    warm-start by interpolating towards the unclipped target relative price
//!    using successive substitution: `r ← (1−α)r + α τ(c(r))` (where `α = 0.5`).
//! 3. Run the Basin trust-region-reflective (TRF) optimizer to minimize the
//!    difference between the current relative prices and the target prices
//!    (`‖r − τ(orders(r))‖²`), bounded by the allowed price range.
//! 4. Polish the result using successive substitution. This simultaneous fixed-point
//!    iteration snaps bounds perfectly, overcoming the interior-reflective limitation
//!    of the TRF algorithm. It also acts as a fallback if the optimizer fails.
//!
//! # Target Prices and Bounds
//!
//! The optimizer targets the unclipped relative price, τ ([`crate::unclipped_target_relative_price`]).
//! This avoids zero-gradient regions during the solve. For a formal breakdown of the
//! game economics, the strict `clamp()` boundary definition, and why successive
//! substitution is required to hit exact market limits, see
//! [`docs/prices-equilibrium.md`](../../../../docs/prices-equilibrium.md).
//!
//! [`equilibrate`] returns a compact [`SolveOutcome`] (goods, residual, relative,
//! building revenues). [`solve`] packages that into a full [`PricesResult`] via
//! [`crate::report`] for CLI / UI / SQL. Planning should prefer
//! [`equilibrate_cached`] with a patched [`crate::ShopCache`].
//!
//! Downstream: [`PricesResult`] feeds `vic3-api` JSON; see the crate root docs.
//! Formulation (nested today, joint/star target): [`docs/prices-equilibrium.md`](../../../../docs/prices-equilibrium.md).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use basin::{
    BoxConstraints, CostFunction, DenseMatrix, Executor, Jacobian, Residual, TerminationReason, Trf,
};
use vic3_defs::{GameDefs, GoodId, GoodsVec};

use crate::consumption::add_wage_bins;
use crate::formula::{local_price, target_price, unclipped_target_relative_price};
use crate::report::{building_revenues_from_cache, report_from_solve};
use crate::result::{
    GoodPrice, PricesResult, SolveOpts, SolveOutcome, SolveStats, SolveStatus, SolveStrategy,
};
use crate::shop_cache::{ShopCache, StateShop};
use crate::world::World;

const WARM_START_ALPHA: f64 = 0.5;
const FD_STEP: f64 = 1e-7;
const LOCAL_ITERS: u32 = 16;
const LOCAL_EPS: f64 = 1e-10;

/// Find relative prices `r` minimizing `‖r − τ(orders(r))‖²`
/// with box bounds `r ∈ [1 − PRICE_RANGE, 1 + PRICE_RANGE]`.
///
/// * `world` — buildings, pops, trade, infra (cold path builds a [`ShopCache`]).
/// * `defs` — goods, price range, recipes.
/// * `opts` — iteration limits and optional `warm_rel`.
///
/// Returns a compact [`SolveOutcome`] without building the full UI/SQL tables.
/// Use [`solve`] when callers need [`PricesResult`].
pub fn equilibrate(world: &World, defs: &GameDefs, opts: SolveOpts) -> SolveOutcome {
    let cache = ShopCache::from_world(world, defs);
    equilibrate_from_cache(&cache, defs, opts).0
}

/// Re-solve from an existing [`ShopCache`] (planning hot path).
///
/// * `cache` — baseline or delta-patched shops; not mutated.
/// * `defs` — goods catalog and price range.
/// * `opts` — iteration limits and optional warm start.
pub fn equilibrate_cached(cache: &ShopCache, defs: &GameDefs, opts: SolveOpts) -> SolveOutcome {
    equilibrate_from_cache(cache, defs, opts).0
}

/// Full public solve: [`equilibrate`] then [`crate::report`].
///
/// * `world` — same as [`equilibrate`].
/// * `defs` — game definitions.
/// * `opts` — solver options.
pub fn solve(world: &World, defs: &GameDefs, opts: SolveOpts) -> PricesResult {
    let cache = ShopCache::from_world(world, defs);
    let (outcome, snapshot) = equilibrate_from_cache(&cache, defs, opts);
    report_from_solve(world, defs, &outcome, snapshot.as_ref())
}

/// Apply a building-level delta and re-solve. Employment (`staffing`) stays frozen.
///
/// Equivalent to cloning via [`World::with_extra_levels`] then [`solve`]. Prefer
/// [`crate::preview`] with a [`crate::WorldDelta`] when swapping PMs or targeting
/// a building id.
pub fn what_if(
    world: &World,
    defs: &GameDefs,
    delta: &crate::result::WhatIfOpts,
    opts: SolveOpts,
) -> PricesResult {
    let next = world.with_extra_levels(delta.building_type_id, delta.extra_levels);
    solve(&next, defs, opts)
}

/// Run NLS using shops already assembled in `cache`.
///
/// Steps: validate goods → build residual → warm start → Basin TRF → polish →
/// evaluate → building revenues from `cache.buildings`.
///
/// [`SolveStrategy::Joint`] currently aliases [`SolveStrategy::Nested`].
fn equilibrate_from_cache(
    cache: &ShopCache,
    defs: &GameDefs,
    opts: SolveOpts,
) -> (SolveOutcome, Option<ShopSnapshot>) {
    match opts.strategy {
        SolveStrategy::Nested | SolveStrategy::Joint => equilibrate_nested(cache, defs, opts),
    }
}

/// Generates zeroed out `SolveStats` for when a market has no goods.
fn empty_stats(strategy: SolveStrategy) -> SolveStats {
    SolveStats {
        strategy,
        param_dim: 0,
        n_residual_evals: 0,
        n_jacobian_evals: 0,
    }
}

/// Core solver implementation using a nested approach.
///
/// In this approach, only the global market relative prices (`rel`) are treated as independent
/// variables by the TRF solver. Local state prices are resolved via an inner successive
/// substitution loop during every residual evaluation.
fn equilibrate_nested(
    cache: &ShopCache,
    defs: &GameDefs,
    opts: SolveOpts,
) -> (SolveOutcome, Option<ShopSnapshot>) {
    let strategy = opts.strategy;
    let goods = market_goods(&cache.base_prices);
    if goods.is_empty() {
        return (
            SolveOutcome {
                goods: Vec::new(),
                residual: 0.0,
                status: SolveStatus::Converged,
                relative: Vec::new(),
                building_revenues: Vec::new(),
                stats: empty_stats(strategy),
            },
            None,
        );
    }

    let bases: Vec<f64> = goods.iter().map(|&idx| cache.base_prices[idx]).collect();
    if bases.iter().any(|b| *b <= 0.0) {
        return (
            SolveOutcome {
                goods: Vec::new(),
                residual: f64::INFINITY,
                status: SolveStatus::Failed,
                relative: Vec::new(),
                building_revenues: Vec::new(),
                stats: empty_stats(strategy),
            },
            None,
        );
    }

    let price_range = defs.price_range.max(0.0);
    let n = goods.len();
    let n_residual_evals = Cell::new(0);
    let n_jacobian_evals = Cell::new(0);
    let problem = PriceResidual {
        defs,
        goods: &goods,
        bases: &bases,
        price_range,
        lower: vec![1.0 - price_range; n],
        upper: vec![1.0 + price_range; n],
        cache: Arc::new(cache.clone()),
        n_residual_evals: &n_residual_evals,
        n_jacobian_evals: &n_jacobian_evals,
    };

    let mut rel = vec![1.0; n];
    let mut warm_iters = opts.max_iters.clamp(1, 16);
    if let Some(warm) = opts.warm_rel.as_ref() {
        if warm.len() == n {
            rel.clone_from(warm);
            problem.clamp_rel(&mut rel);
            warm_iters = 0;
        }
    }
    if warm_iters > 0 {
        problem.damp_toward_formula(&mut rel, WARM_START_ALPHA, warm_iters);
    }

    let mut termination = None;
    if price_range > 0.0 {
        let basin_iters = u64::from(opts.max_iters.saturating_sub(warm_iters)).max(1);
        match Executor::from_start(problem.clone(), Trf::new(), rel.clone())
            .max_iter(basin_iters)
            .run()
        {
            Ok(outcome) => {
                rel.clone_from(outcome.param());
                termination = Some(outcome.reason);
            }
            Err(e) => match e {},
        }
    }

    // TRF stays strictly inside the box; successive substitution may sit on a
    // closed bound and is also the fallback after SolverFailed.
    let failed = matches!(termination, Some(TerminationReason::SolverFailed));
    let polish = if failed { opts.max_iters } else { warm_iters };
    problem.damp_toward_formula(&mut rel, WARM_START_ALPHA, polish);
    problem.clamp_rel(&mut rel);

    let (rows, residual, snapshot) = problem.evaluate(&rel);
    let status = if residual < opts.residual_eps {
        SolveStatus::Converged
    } else if failed {
        SolveStatus::Failed
    } else {
        SolveStatus::MaxIters
    };

    let building_revenues = building_revenues_from_cache(cache, defs, &rows, Some(&snapshot));
    let stats = SolveStats {
        strategy,
        param_dim: rel.len(),
        n_residual_evals: n_residual_evals.get(),
        n_jacobian_evals: n_jacobian_evals.get(),
    };
    (
        SolveOutcome {
            goods: rows,
            residual,
            status,
            relative: rel,
            building_revenues,
            stats,
        },
        Some(snapshot),
    )
}

/// Goods with positive base price (NLS unknowns).
///
/// * `base_prices` — aligned to `defs.goods_order`.
fn market_goods(base_prices: &GoodsVec) -> Vec<GoodId> {
    base_prices
        .iter_indexed()
        .filter(|(_, base)| *base > 0.0)
        .map(|(idx, _)| idx)
        .collect()
}

struct SettleScratch {
    local: GoodsVec,
    pop_buy: GoodsVec,
    next: GoodsVec,
}

impl SettleScratch {
    fn new(n: usize) -> Self {
        Self {
            local: GoodsVec::zeros(n),
            pop_buy: GoodsVec::zeros(n),
            next: GoodsVec::zeros(n),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ShopSnapshot {
    pub(crate) world_pop_buy: GoodsVec,
    pub(crate) local_by_state: BTreeMap<u32, GoodsVec>,
    pub(crate) pop_buy_by_state: BTreeMap<u32, GoodsVec>,
}

/// NLS residual: relative prices vs unclipped τ from shop orders + pop settle.
///
/// * `defs` — labels / good ids for output rows.
/// * `goods` / `bases` — priced goods and their base prices (NLS unknowns).
/// * `price_range` — Vic3 clamp width (box on `r`; τ itself is unclipped).
/// * `lower` / `upper` — box bounds on relative prices.
/// * `cache` — frozen shops/orders (Arc so Basin `Clone` only bumps refcount).
/// * `n_residual_evals` / `n_jacobian_evals` — shared Basin call counters.
#[derive(Clone)]
struct PriceResidual<'a> {
    defs: &'a GameDefs,
    goods: &'a [GoodId],
    bases: &'a [f64],
    price_range: f64,
    lower: Vec<f64>,
    upper: Vec<f64>,
    cache: Arc<ShopCache>,
    n_residual_evals: &'a Cell<u64>,
    n_jacobian_evals: &'a Cell<u64>,
}

impl PriceResidual<'_> {
    /// Map relative prices `rel` to absolute prices using base prices in the cache.
    /// Converts a slice of relative prices (one per market good) into a full `GoodsVec` of market prices.
    ///
    /// # Arguments
    /// * `rel` - A slice of relative prices `r_g` corresponding to the goods in `self.goods`.
    ///
    /// # Returns
    /// A `GoodsVec` containing the absolute market price `p_g = base_g * r_g` for each good.
    fn prices_from_rel(&self, rel: &[f64]) -> GoodsVec {
        let mut prices = self.cache.base_prices.clone();
        for (&good, (&base, &r)) in self.goods.iter().zip(self.bases.iter().zip(rel)) {
            prices[good] = base * r;
        }
        prices
    }

    /// Iterate local settle for one state until local prices stabilize.
    ///
    /// * `shop` — that state’s frozen orders + wage bins.
    /// * `market` — current market absolute prices.
    /// * `scratch` - reusable buffers for local / pop_buy / next.
    ///
    /// Computes the local market prices, wage shifts, and resulting pop buy volumes for a single state.
    ///
    /// # Arguments
    /// * `shop` - The state's local shop cache containing frozen data and wealth bins.
    /// * `market` - The absolute market prices `p_g` for all goods.
    /// * `scratch` - A scratchpad for reusing allocations during evaluation.
    ///
    /// # Side Effects
    /// Mutates `scratch.locals` with the blended local prices and `scratch.pop_buy` with the resulting pop consumption.
    fn settle_state(&self, shop: &StateShop, market: &GoodsVec, scratch: &mut SettleScratch) {
        let n = self.cache.base_prices.len();
        scratch.local.copy_from(market);
        scratch.pop_buy.copy_from(&shop.frozen_pop_buy);
        for _ in 0..LOCAL_ITERS {
            scratch.pop_buy.copy_from(&shop.frozen_pop_buy);
            add_wage_bins(
                &mut scratch.pop_buy,
                &shop.wage_bins,
                &scratch.local,
                &self.cache.base_prices,
                &self.cache.units,
                1.0,
            );
            let mut delta = 0.0_f64;
            for i in 0..n {
                let good = GoodId::from_usize(i);
                let buy = shop.frozen_buy[good] + scratch.pop_buy[good];
                let sell = shop.frozen_sell[good];
                let base = self.cache.base_prices[good];
                // Calculate the unclipped local price target, blend it with the global market
                // price using MAPI, and finally clamp the resulting local price to strictly
                // obey the game's price limits.
                let state_target = target_price(base, buy, sell, self.price_range);
                let blended = local_price(shop.mapi, market[good], state_target);
                let local = blended.clamp(
                    base * (1.0 - self.price_range),
                    base * (1.0 + self.price_range),
                );
                delta = delta.max((local - scratch.local[good]).abs());
                scratch.next[good] = local;
            }
            scratch.local.copy_from(&scratch.next);
            if delta < LOCAL_EPS {
                break;
            }
        }
    }

    /// World pop buy at `market` prices (stateless wages + access-scaled state settles).
    /// Computes the aggregate worldwide pop consumption for all goods at the given market prices.
    ///
    /// # Arguments
    /// * `market` - The absolute market prices `p_g` for all goods.
    /// * `scratch` - A scratchpad for reusing allocations.
    ///
    /// # Returns
    /// A `GoodsVec` summing the pop buy volumes across all states.
    fn world_pop_buy_at(&self, market: &GoodsVec, scratch: &mut SettleScratch) -> GoodsVec {
        let mut buy = self.cache.frozen_pop_buy.clone();
        add_wage_bins(
            &mut buy,
            &self.cache.stateless_wage_bins,
            market,
            &self.cache.base_prices,
            &self.cache.units,
            1.0,
        );
        for shop in &self.cache.shops {
            if shop.wage_bins.is_empty() {
                continue;
            }
            self.settle_state(shop, market, scratch);
            for (good, quantity) in scratch.pop_buy.iter_indexed() {
                buy.add(good, shop.access * (quantity - shop.frozen_pop_buy[good]));
            }
        }
        buy
    }

    /// Full settle snapshot: world pop buy plus per-state local / pop_buy maps.
    /// Takes a full snapshot of the local prices and pop buy volumes across all states at the given market prices.
    ///
    /// # Arguments
    /// * `market` - The absolute market prices `p_g` for all goods.
    /// * `scratch` - A scratchpad for reusing allocations.
    ///
    /// # Returns
    /// A `ShopSnapshot` containing the local prices per state and the worldwide aggregate pop consumption.
    fn snapshot_at(&self, market: &GoodsVec, scratch: &mut SettleScratch) -> ShopSnapshot {
        let mut world_pop_buy = self.cache.frozen_pop_buy.clone();
        add_wage_bins(
            &mut world_pop_buy,
            &self.cache.stateless_wage_bins,
            market,
            &self.cache.base_prices,
            &self.cache.units,
            1.0,
        );
        let mut local_by_state = BTreeMap::new();
        let mut pop_buy_by_state = BTreeMap::new();
        for shop in &self.cache.shops {
            self.settle_state(shop, market, scratch);
            for (good, quantity) in scratch.pop_buy.iter_indexed() {
                world_pop_buy.add(good, shop.access * (quantity - shop.frozen_pop_buy[good]));
            }
            local_by_state.insert(shop.id, scratch.local.clone());
            pop_buy_by_state.insert(shop.id, scratch.pop_buy.clone());
        }
        ShopSnapshot {
            world_pop_buy,
            local_by_state,
            pop_buy_by_state,
        }
    }

    /// Helper to compute worldwide pop consumption directly from absolute market prices.
    ///
    /// # Arguments
    /// * `prices` - The absolute market prices `p_g` for all goods.
    /// * `scratch` - A scratchpad for reusing allocations.
    ///
    /// # Returns
    /// A `GoodsVec` with the total pop buy volumes.
    fn pop_buy_at(&self, prices: &GoodsVec, scratch: &mut SettleScratch) -> GoodsVec {
        self.world_pop_buy_at(prices, scratch)
    }

    /// Unclipped target relative prices τ from orders at pop demand implied by `rel`.
    /// Computes the unclipped target relative price (τ) for each market good based on current orders.
    ///
    /// # Arguments
    /// * `rel` - The current relative prices `r_g` to evaluate at.
    /// * `scratch` - A scratchpad for reusing allocations.
    ///
    /// # Returns
    /// A vector of unclipped target relative prices `τ_g` for each market good.
    fn formula_rel(&self, rel: &[f64], scratch: &mut SettleScratch) -> Vec<f64> {
        let prices = self.prices_from_rel(rel);
        let pop_buy = self.pop_buy_at(&prices, scratch);
        self.goods
            .iter()
            .map(|&id| {
                let buy = self.cache.frozen_buy[id] + pop_buy[id];
                let sell = self.cache.frozen_sell[id];
                unclipped_target_relative_price(buy, sell, self.price_range)
            })
            .collect()
    }

    /// Computes the mathematical residual `R_g = r_g - τ_g` for all market goods.
    ///
    /// # Arguments
    /// * `rel` - The current relative prices `r_g`.
    /// * `scratch` - A scratchpad for reusing allocations.
    ///
    /// # Returns
    /// A vector of residuals `R_g` corresponding to each market good.
    fn residual_at(&self, rel: &[f64], scratch: &mut SettleScratch) -> Vec<f64> {
        self.formula_rel(rel, scratch)
            .into_iter()
            .zip(rel)
            .map(|(formula, r)| r - formula)
            .collect()
    }

    /// Clamps the given relative prices `r_g` into the solver's valid box `[1 - ρ, 1 + ρ]`.
    ///
    /// # Arguments
    /// * `rel` - A mutable slice of relative prices `r_g` to be clamped in place.
    fn clamp_rel(&self, rel: &mut [f64]) {
        for (i, r) in rel.iter_mut().enumerate() {
            *r = r.clamp(self.lower[i], self.upper[i]);
        }
    }

    /// Warms up relative prices via damped successive substitution `r ← (1−α)r + α τ(c(r))`.
    ///
    /// # Arguments
    /// * `rel` - A mutable slice of relative prices to start from and update in place.
    /// * `alpha` - The damping factor (e.g., `0.5`).
    /// * `iters` - The number of substitution iterations to perform.
    fn damp_toward_formula(&self, rel: &mut [f64], alpha: f64, iters: u32) {
        let mut scratch = SettleScratch::new(self.cache.base_prices.len());
        for _ in 0..iters {
            let formula = self.formula_rel(rel, &mut scratch);
            for (r, f) in rel.iter_mut().zip(formula) {
                *r = (1.0 - alpha) * *r + alpha * f;
            }
            self.clamp_rel(rel);
        }
    }

    /// Full evaluation of the market state at the given relative prices `r_g`.
    ///
    /// This resolves the local state, aggregates supply/demand, and yields the final rows and residuals.
    ///
    /// # Arguments
    /// * `rel` - A slice of relative prices `r_g` corresponding to each market good.
    ///
    /// # Returns
    /// A tuple containing:
    /// 1. A vector of `GoodPrice` rows summarizing base, price, buy, and sell volumes per good.
    /// 2. The scalar objective value (the root sum of squared residuals `‖R‖`).
    /// 3. The `ShopSnapshot` containing local price/consumption details across states.
    fn evaluate(&self, rel: &[f64]) -> (Vec<GoodPrice>, f64, ShopSnapshot) {
        let mut scratch = SettleScratch::new(self.cache.base_prices.len());
        let prices = self.prices_from_rel(rel);
        let snapshot = self.snapshot_at(&prices, &mut scratch);
        let pop_buy = &snapshot.world_pop_buy;
        let residual = self
            .goods
            .iter()
            .zip(rel.iter())
            .map(|(&id, rrel)| {
                let buy = self.cache.frozen_buy[id] + pop_buy[id];
                let sell = self.cache.frozen_sell[id];
                let formula = unclipped_target_relative_price(buy, sell, self.price_range);
                rrel - formula
            })
            .map(|x| x * x)
            .sum::<f64>()
            .sqrt();
        let rows = self
            .goods
            .iter()
            .zip(self.bases.iter().zip(rel.iter()))
            .filter_map(|(&id, (base, rrel))| {
                let good_id = self.defs.good_by_index(id)?;
                let buy = self.cache.frozen_buy[id] + pop_buy[id];
                let sell = self.cache.frozen_sell[id];
                Some(GoodPrice {
                    name: good_id.to_string(),
                    label: self.defs.display_label(good_id),
                    base: *base,
                    price: *base * *rrel,
                    buy,
                    sell,
                })
            })
            .collect();
        (rows, residual, snapshot)
    }
}

impl CostFunction for PriceResidual<'_> {
    type Param = Vec<f64>;
    type Output = f64;
    type Error = Infallible;

    fn cost(&self, param: &Vec<f64>) -> Result<f64, Infallible> {
        self.n_residual_evals.set(self.n_residual_evals.get() + 1);
        let mut scratch = SettleScratch::new(self.cache.base_prices.len());
        Ok(0.5
            * self
                .residual_at(param, &mut scratch)
                .iter()
                .map(|x| x * x)
                .sum::<f64>())
    }
}

impl Residual for PriceResidual<'_> {
    type Param = Vec<f64>;
    type Output = Vec<f64>;
    type Error = Infallible;

    fn residual(&self, param: &Vec<f64>) -> Result<Vec<f64>, Infallible> {
        self.n_residual_evals.set(self.n_residual_evals.get() + 1);
        let mut scratch = SettleScratch::new(self.cache.base_prices.len());
        Ok(self.residual_at(param, &mut scratch))
    }
}

impl Jacobian for PriceResidual<'_> {
    type Jacobian = DenseMatrix<f64>;

    fn jacobian(&self, param: &Vec<f64>) -> Result<DenseMatrix<f64>, Infallible> {
        self.n_jacobian_evals.set(self.n_jacobian_evals.get() + 1);
        let mut scratch = SettleScratch::new(self.cache.base_prices.len());
        let r0 = self.residual_at(param, &mut scratch);
        let n = param.len();
        let m = r0.len();
        let mut data = vec![0.0; m * n];
        for j in 0..n {
            let h = FD_STEP.max(FD_STEP * param[j].abs());
            let mut stepped = param.clone();
            let (x1, denom) = if param[j] + h <= self.upper[j] {
                stepped[j] = param[j] + h;
                (self.residual_at(&stepped, &mut scratch), h)
            } else {
                stepped[j] = param[j] - h;
                (self.residual_at(&stepped, &mut scratch), -h)
            };
            for i in 0..m {
                data[i * n + j] = (x1[i] - r0[i]) / denom;
            }
        }
        Ok(DenseMatrix::from_row_slice(m, n, &data))
    }
}

impl BoxConstraints for PriceResidual<'_> {
    fn lower(&self) -> &Vec<f64> {
        &self.lower
    }

    fn upper(&self) -> &Vec<f64> {
        &self.upper
    }
}
