//! Joint (coupled) market + pure-state price equilibrium solver.
//!
//! Unlike [`crate::solve::equilibrate_nested`], which alternates market clearing
//! with an inner local settle, this module solves for worldwide relative prices
//! `r_g` and every state's **pure-state** absolute prices `σ_{s,g}` in one
//! bound-constrained NLS. Blended local prices (what pops pay) are derived:
//!
//! ```text
//! p_{s,g} = local_price(m_s, market(r)_g, σ_{s,g})
//! ```
//!
//! Basin's trust-region-reflective solver (`Trf`) drives the combined residual
//! to zero.
//!
//! # State vector
//!
//! ```text
//! x = [ r_0, …, r_{G-1},  σ_{0,0}, …, σ_{0,G-1},  σ_{1,0}, …, σ_{S-1,G-1} ]
//! ```
//!
//! Market rows use relative prices; pure-state rows use absolute prices with
//! residual scaled by `1 / base_g` so both blocks have comparable magnitude.
//! Box bounds on `r` and `σ` enforce ±`PRICE_RANGE`; order-implied targets stay
//! unclipped (same pattern as the nested market residual).
//!
//! # Jacobian
//!
//! Market rows couple all states through aggregate pop consumption, so batched
//! finite-difference perturbations of pure-state prices cannot be reused for
//! market derivatives. The implementation keeps batched FD for separable
//! pure-state rows and derives market-row columns analytically from per-state
//! pop-buy deltas.

use std::cell::Cell;
use std::convert::Infallible;
use std::sync::Arc;

use basin::{BoxConstraints, CostFunction, Executor, Jacobian, Residual, TerminationReason, Trf};
use faer::col::Col;
use faer::sparse::{SparseColMat, Triplet};
use vic3_defs::{GameDefs, GoodId, GoodsVec};

use crate::consumption::add_wage_bins;
use crate::formula::{local_price, target_price, unclipped_target_relative_price};
use crate::profile_markers::{self, BasinIterTracker};
use crate::result::{GoodPrice, SolveOpts, SolveOutcome, SolveStats, SolveStatus};
use crate::shop_cache::ShopCache;
use crate::solve::{empty_stats, market_goods, ShopSnapshot};

/// Central finite-difference step for Jacobian columns that use explicit FD.
const FD_STEP: f64 = 1e-7;

/// Basin problem: joint market + per-state pure-state price residuals.
///
/// Implements [`CostFunction`], [`Residual`], [`Jacobian`], and [`BoxConstraints`]
/// for the trust-region solver. Counter cells track evaluation counts for
/// [`SolveStats`].
#[derive(Clone)]
struct PriceResidualJoint<'a> {
    defs: &'a GameDefs,
    goods: &'a [GoodId],
    bases: &'a [f64],
    price_range: f64,
    lower: Col<f64>,
    upper: Col<f64>,
    cache: Arc<ShopCache>,
    n_residual_evals: &'a Cell<u64>,
    n_jacobian_evals: &'a Cell<u64>,
    basin_iter: &'a BasinIterTracker,
}

impl<'a> PriceResidualJoint<'a> {
    /// Returns the number of priced goods in the market.
    fn n_goods(&self) -> usize {
        self.goods.len()
    }

    /// Returns the number of states with non-empty local shops.
    fn n_states(&self) -> usize {
        self.cache.shops.len()
    }

    /// Returns the total dimension of the joint state vector `x`.
    ///
    /// `x` is market relative prices `r_g` (length `G`), then pure-state absolute
    /// prices `σ_{s,g}` for each state (length `S * G`). Blended locals are not
    /// free unknowns.
    fn state_dim(&self) -> usize {
        let g = self.n_goods();
        let s = self.n_states();
        g + s * g
    }

    /// Converts a slice of market relative prices into a full `GoodsVec` of absolute market prices.
    ///
    /// # Arguments
    /// * `rel` - A slice of market relative prices `r_g`.
    ///
    /// # Returns
    /// A `GoodsVec` of absolute market prices `p_g = base_g * r_g`.
    fn prices_from_rel(&self, rel: &[f64]) -> GoodsVec {
        let mut prices = self.cache.base_prices.clone();
        for (&good, (&base, &r)) in self.goods.iter().zip(self.bases.iter().zip(rel)) {
            prices[good] = base * r;
        }
        prices
    }

    /// Evaluates the full joint residual block `R(x)`.
    ///
    /// Market block: `R_g = r_g - τ_mkt`. Pure-state block:
    /// `R_{s,g} = (σ_{s,g} - τ_state(orders at p_{s})) / base_g` with
    /// `p_s = blend(m_s, market(r), σ_s)`.
    fn residual_at(&self, x: &Col<f64>) -> Col<f64> {
        self.eval_residual(x, None)
    }

    /// Evaluates `R(x)` and optionally records per-state pop-buy volumes.
    ///
    /// When `out_pop_buys` is `Some`, it must hold `S * G` elements laid out as
    /// `[state0_good0, …, state0_good_{G-1}, state1_good0, …]`. Values are stored
    /// in the **compact market-good index** `i ∈ 0..G`, not the full `GoodId`
    /// index — `market_goods` may exclude goods with nonpositive base prices, so
    /// `G` can be smaller than `base_prices.len()`.
    fn eval_residual(&self, x: &Col<f64>, mut out_pop_buys: Option<&mut [f64]>) -> Col<f64> {
        let g = self.n_goods();
        let mut res = Col::zeros(self.state_dim());

        // x[0..g] contains the global market relative prices (price / base_price).
        let market_rel: Vec<f64> = (0..g).map(|i| x[i]).collect();
        let market_prices = self.prices_from_rel(&market_rel);

        // Stateless consumption at market prices.
        let mut world_pop_buy = self.cache.frozen_pop_buy.clone();
        add_wage_bins(
            &mut world_pop_buy,
            &self.cache.stateless_wage_bins,
            &market_prices,
            &self.cache.base_prices,
            &self.cache.units,
            1.0,
        );

        let mut pop_buy_scratch = GoodsVec::zeros(self.cache.base_prices.len());
        let mut local_prices_scratch = GoodsVec::zeros(self.cache.base_prices.len());

        // Pure-state residual R^{σ}_{s,g} for each state and good.
        for (s_idx, shop) in self.cache.shops.iter().enumerate() {
            local_prices_scratch.copy_from(&self.cache.base_prices);
            let s_offset = g + s_idx * g;

            // Derive blended local prices from free pure-state σ_{s,g}.
            for i in 0..g {
                let good = self.goods[i];
                let sigma = x[s_offset + i];
                local_prices_scratch[good] = local_price(shop.mapi, market_prices[good], sigma);
            }

            // Pops shop at blended locals, not at σ.
            pop_buy_scratch.copy_from(&shop.frozen_pop_buy);
            add_wage_bins(
                &mut pop_buy_scratch,
                &shop.wage_bins,
                &local_prices_scratch,
                &self.cache.base_prices,
                &self.cache.units,
                1.0,
            );

            if let Some(ref mut out) = out_pop_buys {
                let start = s_idx * g;
                for (i, &good) in self.goods.iter().enumerate() {
                    out[start + i] = pop_buy_scratch[good];
                }
            }

            for i in 0..g {
                let good = self.goods[i];
                let buy = shop.frozen_buy[good] + pop_buy_scratch[good];
                let sell = shop.frozen_sell[good];
                let base = self.bases[i];
                let state_target = target_price(base, buy, sell, self.price_range);

                // R^{σ}_{s,g} = (σ_{s,g} - τ_state) / base_g
                res[s_offset + i] = (x[s_offset + i] - state_target) / base;

                world_pop_buy.add(
                    good,
                    shop.access * (pop_buy_scratch[good] - shop.frozen_pop_buy[good]),
                );
            }
        }

        // Global market residual R_g.
        for i in 0..g {
            let good = self.goods[i];
            let buy = self.cache.frozen_buy[good] + world_pop_buy[good];
            let sell = self.cache.frozen_sell[good];
            let target = unclipped_target_relative_price(buy, sell, self.price_range);
            res[i] = market_rel[i] - target;
        }

        res
    }

    /// Full evaluation at `x`: market rows, blended locals, pop buys, snapshot.
    ///
    /// Snapshot stores both free pure-state prices `σ` and derived locals
    /// `p = blend(m, market, σ)` so emit can publish a coherent `StateGood` row.
    fn evaluate(&self, x: &Col<f64>) -> (Vec<GoodPrice>, f64, ShopSnapshot) {
        let g = self.n_goods();
        let market_rel: Vec<f64> = (0..g).map(|i| x[i]).collect();
        let market_prices = self.prices_from_rel(&market_rel);

        let mut world_pop_buy = self.cache.frozen_pop_buy.clone();
        add_wage_bins(
            &mut world_pop_buy,
            &self.cache.stateless_wage_bins,
            &market_prices,
            &self.cache.base_prices,
            &self.cache.units,
            1.0,
        );

        let mut snapshot = ShopSnapshot::default();
        let mut pop_buy_scratch = GoodsVec::zeros(self.cache.base_prices.len());
        let mut local_prices_scratch = GoodsVec::zeros(self.cache.base_prices.len());
        let mut pure_state_scratch = GoodsVec::zeros(self.cache.base_prices.len());

        for (s_idx, shop) in self.cache.shops.iter().enumerate() {
            local_prices_scratch.copy_from(&self.cache.base_prices);
            pure_state_scratch.copy_from(&self.cache.base_prices);
            let s_offset = g + s_idx * g;
            for i in 0..g {
                let good = self.goods[i];
                let sigma = x[s_offset + i];
                pure_state_scratch[good] = sigma;
                local_prices_scratch[good] = local_price(shop.mapi, market_prices[good], sigma);
            }
            snapshot
                .pure_state_by_state
                .insert(shop.id, pure_state_scratch.clone());
            snapshot
                .local_by_state
                .insert(shop.id, local_prices_scratch.clone());

            pop_buy_scratch.copy_from(&shop.frozen_pop_buy);
            add_wage_bins(
                &mut pop_buy_scratch,
                &shop.wage_bins,
                &local_prices_scratch,
                &self.cache.base_prices,
                &self.cache.units,
                1.0,
            );
            snapshot
                .pop_buy_by_state
                .insert(shop.id, pop_buy_scratch.clone());

            for i in 0..g {
                let good = self.goods[i];
                world_pop_buy.add(
                    good,
                    shop.access * (pop_buy_scratch[good] - shop.frozen_pop_buy[good]),
                );
            }
        }
        snapshot.world_pop_buy = world_pop_buy.clone();

        let residual = self
            .residual_at(x)
            .iter()
            .map(|v| v * v)
            .sum::<f64>()
            .sqrt();

        let rows = self
            .goods
            .iter()
            .zip(self.bases.iter().zip(market_rel.iter()))
            .filter_map(|(&id, (base, rrel))| {
                let good_id = self.defs.good_by_index(id)?;
                let buy = self.cache.frozen_buy[id] + world_pop_buy[id];
                let sell = self.cache.frozen_sell[id];
                Some(GoodPrice {
                    name: good_id.to_string(),
                    label: self.defs.display_label(good_id),
                    base: *base,
                    price: base * rrel,
                    buy,
                    sell,
                })
            })
            .collect();

        (rows, residual, snapshot)
    }
}

impl CostFunction for PriceResidualJoint<'_> {
    type Param = Col<f64>;
    type Output = f64;
    type Error = Infallible;

    /// Half the squared residual norm, for Basin's cost-based stopping hooks.
    fn cost(&self, param: &Col<f64>) -> Result<f64, Infallible> {
        self.basin_iter.note_residual_or_cost();
        #[cfg(feature = "profiling-markers")]
        let _span = tracing::info_span!("cost").entered();
        self.n_residual_evals.set(self.n_residual_evals.get() + 1);
        Ok(0.5 * self.residual_at(param).iter().map(|x| x * x).sum::<f64>())
    }
}

impl Residual for PriceResidualJoint<'_> {
    type Param = Col<f64>;
    type Output = Col<f64>;
    type Error = Infallible;

    /// Joint residual vector `R(x)` passed to the TRF solver.
    fn residual(&self, param: &Col<f64>) -> Result<Col<f64>, Infallible> {
        self.basin_iter.note_residual_or_cost();
        #[cfg(feature = "profiling-markers")]
        let _span = tracing::info_span!("residual").entered();
        self.n_residual_evals.set(self.n_residual_evals.get() + 1);
        Ok(self.residual_at(param))
    }
}

impl Jacobian for PriceResidualJoint<'_> {
    type Jacobian = SparseColMat<usize, f64>;

    /// Sparse Jacobian `∂R/∂x` at `param`.
    ///
    /// Market columns (`r_j`) use standard finite differences. Pure-state columns
    /// (`σ_{s,j}`) batch FD for the separable state block and analytical τ
    /// derivatives for market rows (see module docs).
    fn jacobian(&self, param: &Col<f64>) -> Result<SparseColMat<usize, f64>, Infallible> {
        self.basin_iter.begin_jacobian();
        let out = {
            #[cfg(feature = "profiling-markers")]
            let _span = tracing::info_span!("jacobian").entered();
            self.n_jacobian_evals.set(self.n_jacobian_evals.get() + 1);
            let g = self.n_goods();
            let n = self.state_dim();

            let mut base_pop_buys = vec![0.0; self.n_states() * g];
            let r0 = self.eval_residual(param, Some(&mut base_pop_buys));
            let mut triplets = Vec::new();

            // Compute base world_buy for analytical market derivatives
            let mut base_world_buy = vec![0.0; g];
            {
                let market_rel: Vec<f64> = (0..g).map(|i| param[i]).collect();
                let market_prices = self.prices_from_rel(&market_rel);
                let mut world_pop_buy = self.cache.frozen_pop_buy.clone();
                add_wage_bins(
                    &mut world_pop_buy,
                    &self.cache.stateless_wage_bins,
                    &market_prices,
                    &self.cache.base_prices,
                    &self.cache.units,
                    1.0,
                );
                for (i, &good) in self.goods.iter().enumerate() {
                    let mut state_sum = 0.0;
                    for (s, shop) in self.cache.shops.iter().enumerate() {
                        state_sum +=
                            shop.access * (base_pop_buys[s * g + i] - shop.frozen_pop_buy[good]);
                    }
                    base_world_buy[i] =
                        self.cache.frozen_buy[good] + world_pop_buy[good] + state_sum;
                }
            }

            // 1. Perturb market prices r_j
            for j in 0..g {
                let h = FD_STEP.max(FD_STEP * param[j].abs());
                let mut stepped = param.clone();
                let (x1, denom) = if param[j] + h <= self.upper[j] {
                    stepped[j] = param[j] + h;
                    (self.residual_at(&stepped), h)
                } else {
                    stepped[j] = param[j] - h;
                    (self.residual_at(&stepped), -h)
                };

                for i in 0..n {
                    let deriv = (x1[i] - r0[i]) / denom;
                    if deriv.abs() > 1e-12 {
                        triplets.push(Triplet::new(i, j, deriv));
                    }
                }
            }

            let mut stepped_pop_buys = vec![0.0; self.n_states() * g];

            // 2. Perturb pure-state prices σ_{s,j}
            // States are independent for pure-state residuals (R^σ_{s,g}): σ in state A
            // does not affect pop consumption in state B (only through the market hub).
            // Therefore we can perturb σ_{s,j} for ALL states in one batched FD step.
            //
            // The global market residual DOES depend on aggregate consumption. Batched
            // pop-buy deltas plus an analytical τ derivative recover per-state market
            // columns without S separate evaluations.
            for j in 0..g {
                let mut batched_stepped = param.clone();
                let mut denoms = vec![0.0; self.n_states()];
                for (s_idx, denom) in denoms.iter_mut().enumerate() {
                    let idx = g + s_idx * g + j;
                    let h = FD_STEP.max(FD_STEP * param[idx].abs());
                    if param[idx] + h <= self.upper[idx] {
                        batched_stepped[idx] = param[idx] + h;
                        *denom = h;
                    } else {
                        batched_stepped[idx] = param[idx] - h;
                        *denom = -h;
                    }
                }

                let batched_x1 = self.eval_residual(&batched_stepped, Some(&mut stepped_pop_buys));

                for (s_idx, &denom) in denoms.iter().enumerate() {
                    let idx = g + s_idx * g + j;

                    // Affects market residuals: analytical via Δbuy at blended locals.
                    // R_m = r_m - τ(world_buy, world_sell)
                    for (i, &good) in self.goods.iter().enumerate() {
                        let delta_buy =
                            stepped_pop_buys[s_idx * g + i] - base_pop_buys[s_idx * g + i];
                        let actual_delta_buy = delta_buy * self.cache.shops[s_idx].access;

                        if actual_delta_buy.abs() > 1e-12 {
                            let sell = self.cache.frozen_sell[good];
                            let target0 = unclipped_target_relative_price(
                                base_world_buy[i],
                                sell,
                                self.price_range,
                            );
                            let target1 = unclipped_target_relative_price(
                                base_world_buy[i] + actual_delta_buy,
                                sell,
                                self.price_range,
                            );

                            let deriv = -(target1 - target0) / denom;
                            if deriv.abs() > 1e-12 {
                                triplets.push(Triplet::new(i, idx, deriv));
                            }
                        }
                    }

                    // Affects THIS state's pure-state residuals: batched evaluation
                    for i in 0..g {
                        let r_idx = g + s_idx * g + i;
                        let deriv = (batched_x1[r_idx] - r0[r_idx]) / denom;
                        if deriv.abs() > 1e-12 {
                            triplets.push(Triplet::new(r_idx, idx, deriv));
                        }
                    }
                }
            }

            Ok(
                SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &triplets)
                    .unwrap_or_else(|_| panic!("invalid sparse Jacobian triplets: {n}×{n}")),
            )
        };
        self.basin_iter.end_jacobian();
        out
    }
}

impl BoxConstraints for PriceResidualJoint<'_> {
    /// Lower bounds on `x` (relative market prices and absolute pure-state prices).
    fn lower(&self) -> &Col<f64> {
        &self.lower
    }

    /// Upper bounds on `x`.
    fn upper(&self) -> &Col<f64> {
        &self.upper
    }
}

/// Solve market + pure-state prices jointly via Basin TRF.
///
/// Returns the same `(SolveOutcome, Option<ShopSnapshot>)` pair as
/// [`crate::solve::equilibrate_nested`]. Snapshot locals are MAPI blends of
/// market and free pure-state unknowns. `max_iters == 0` evaluates the start
/// point without Basin iterations.
pub(crate) fn equilibrate_joint(
    cache: &ShopCache,
    defs: &GameDefs,
    opts: SolveOpts,
) -> (SolveOutcome, Option<ShopSnapshot>) {
    let strategy = opts.strategy;
    let span = profile_markers::PriceSolveSpan::new(strategy);
    let _enter = span.enter();

    let goods = market_goods(&cache.base_prices);

    // Degenerate markets: nothing to price.
    if goods.is_empty() {
        let out = (
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
        span.record(&out.0);
        return out;
    }

    let bases: Vec<f64> = goods.iter().map(|&idx| cache.base_prices[idx]).collect();
    if bases.iter().any(|b| *b <= 0.0) {
        let out = (
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
        span.record(&out.0);
        return out;
    }

    let price_range = defs.price_range.max(0.0);
    let g = goods.len();
    let s = cache.shops.len();
    let n = g + s * g;

    let mut lower = Col::zeros(n);
    let mut upper = Col::zeros(n);

    // Market bounds
    for (i, _) in bases.iter().enumerate() {
        lower[i] = 1.0 - price_range;
        upper[i] = 1.0 + price_range;
    }

    // Pure-state σ bounds (same ±PRICE_RANGE band as game state prices).
    for s_idx in 0..s {
        for (i, &base) in bases.iter().enumerate() {
            let idx = g + s_idx * g + i;
            lower[idx] = base * (1.0 - price_range);
            upper[idx] = base * (1.0 + price_range);
        }
    }

    let mut x = Col::zeros(n);
    // Warm-start market relative prices when the caller supplies a matching vector.
    let use_warm = opts.warm_rel.as_ref().is_some_and(|w| w.len() == g);
    for (i, &_base) in bases.iter().enumerate() {
        x[i] = if use_warm {
            opts.warm_rel.as_ref().unwrap()[i].clamp(lower[i], upper[i])
        } else {
            1.0
        };
    }
    for s_idx in 0..s {
        for (i, &base) in bases.iter().enumerate() {
            let idx = g + s_idx * g + i;
            // Pure-state σ starts at base (relative price 1.0).
            x[idx] = base;
        }
    }

    let n_residual_evals = Cell::new(0);
    let n_jacobian_evals = Cell::new(0);
    let basin_iter = BasinIterTracker::new();
    let problem = PriceResidualJoint {
        defs,
        goods: &goods,
        bases: &bases,
        price_range,
        lower,
        upper,
        cache: Arc::new(cache.clone()),
        n_residual_evals: &n_residual_evals,
        n_jacobian_evals: &n_jacobian_evals,
        basin_iter: &basin_iter,
    };

    let basin_iters = u64::from(opts.max_iters);
    let result = Executor::from_start(problem.clone(), Trf::new(), x.clone())
        .max_iter(basin_iters)
        .run();

    let outcome = match result {
        Ok(o) => o,
        Err(e) => match e {},
    };

    x.clone_from(outcome.param());
    let (rows, residual, snapshot) = problem.evaluate(&x);
    let building_revenues =
        crate::report::building_revenues_from_cache(cache, defs, &rows, Some(&snapshot));

    // Match nested solver: Converged only when ‖R‖ < eps; otherwise Failed or MaxIters.
    let status = match outcome.reason {
        TerminationReason::SolverFailed => SolveStatus::Failed,
        TerminationReason::MaxIter if residual > opts.residual_eps => SolveStatus::MaxIters,
        _ if residual <= opts.residual_eps => SolveStatus::Converged,
        _ => SolveStatus::MaxIters,
    };

    basin_iter.close();
    let out = (
        SolveOutcome {
            goods: rows,
            residual,
            status,
            relative: (0..g).map(|i| x[i]).collect(),
            building_revenues,
            stats: SolveStats {
                strategy,
                param_dim: n,
                n_residual_evals: n_residual_evals.get(),
                n_jacobian_evals: n_jacobian_evals.get(),
            },
        },
        Some(snapshot),
    );
    span.record(&out.0);
    out
}

#[cfg(test)]
mod tests {
    //! White-box tests for the joint residual and its optimized Jacobian.

    use super::*;
    use crate::shop_cache::ShopCache;
    use crate::World;
    use faer::Col;
    use std::path::PathBuf;

    /// Toy economy defs fixture (shared with integration tests).
    fn toy_defs_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures/toy_economy")
    }

    /// Plaintext save for the toy economy fixture.
    fn toy_save_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-load/tests/fixtures/toy_economy.txt")
    }

    /// The optimized Jacobian must match a naive O(n²) finite-difference reference.
    ///
    /// This guards the batched local-price perturbation + analytical market-row
    /// shortcut against regressions. Integration tests only check that the solver
    /// reaches the right equilibrium end-to-end (black box).
    #[test]
    fn test_analytical_jacobian_matches_finite_difference() {
        let defs = vic3_defs::load_from_path(toy_defs_root()).expect("toy economy defs");
        let save = vic3_load::load_path(toy_save_path(), vic3_load::empty_tokens())
            .expect("toy economy save");
        let world = World::from_save(&save, &defs);

        let cache = Arc::new(ShopCache::from_world(&world, &defs));
        let goods = market_goods(&cache.base_prices);
        let bases: Vec<f64> = goods.iter().map(|&id| cache.base_prices[id]).collect();
        let g = goods.len();
        let n = g + cache.shops.len() * g;

        let upper = Col::from_fn(n, |_| 1000.0);
        let lower = Col::from_fn(n, |_| 0.001);

        let res_evals = Cell::new(0);
        let jac_evals = Cell::new(0);
        let basin_iter = BasinIterTracker::new();

        let problem = PriceResidualJoint {
            defs: &defs,
            cache: cache.clone(),
            bases: &bases,
            goods: &goods,
            upper,
            lower,
            price_range: defs.price_range,
            n_residual_evals: &res_evals,
            n_jacobian_evals: &jac_evals,
            basin_iter: &basin_iter,
        };

        let x0 = Col::from_fn(n, |_| 1.5);

        let jac_analytical = problem.jacobian(&x0).unwrap();

        // Reference: perturb each coordinate independently (no batching tricks).
        let r0 = problem.residual_at(&x0);
        let mut expected_triplets = Vec::new();
        let fd_step = FD_STEP;

        for j in 0..n {
            let mut x1 = x0.clone();
            let mut h = fd_step.max(fd_step * x0[j].abs());
            if x0[j] + h > problem.upper[j] {
                h = -h;
            }
            x1[j] += h;

            let r1 = problem.residual_at(&x1);

            for i in 0..n {
                let deriv = (r1[i] - r0[i]) / h;
                if deriv.abs() > 1e-12 {
                    expected_triplets.push((i, j, deriv));
                }
            }
        }

        // Every nonzero FD entry must appear in the analytical matrix.
        for &(i, j, expected_val) in &expected_triplets {
            let col_range = jac_analytical.col_ptr()[j]..jac_analytical.col_ptr()[j + 1];
            let mut analytical_val = 0.0;
            for ptr in col_range {
                if jac_analytical.row_idx()[ptr] == i {
                    analytical_val = jac_analytical.val()[ptr];
                    break;
                }
            }

            // Allow a tiny margin of error due to floating point precision / analytical vs FD.
            let diff = (analytical_val - expected_val).abs();
            assert!(
                diff < 1e-5,
                "Jacobian mismatch at ({}, {}): analytical={} expected={} (diff={})",
                i,
                j,
                analytical_val,
                expected_val,
                diff
            );
        }
    }
}
