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
//!
//! The Jacobian is stored as [`ArrowheadMat`](basin_arrowhead::ArrowheadMat) so
//! Basin [`Trf`](basin::Trf) can form and solve damped Gram systems in
//! **`O(S·G³)`** via hub Schur + Woodbury instead of building a sparse `n×n`
//! `JᵀJ`. See [`basin_arrowhead`] for the block layout and trait implementations.

use std::cell::Cell;
use std::convert::Infallible;
use std::sync::Arc;

use basin::{
    BoxConstraints, CostFunction, Executor, Jacobian, NoImprovement, Residual, TerminationReason,
    Trf,
};
use basin_arrowhead::ArrowheadMat;
use faer::col::Col;
use vic3_defs::{GameDefs, GoodId, GoodsVec};

use crate::consumption::add_wage_bins;
use crate::formula::{local_price, target_price, unclipped_target_relative_price};
use crate::result::{GoodPrice, SolveOpts, SolveOutcome, SolveStats, SolveStatus};
use crate::shop_cache::ShopCache;
use crate::solve::{
    empty_stats, kkt_tol, market_goods, ShopSnapshot, STALL_PATIENCE, STALL_REL_TOL,
};

/// Finite-difference step for Jacobian columns that use explicit FD.
///
/// As in the nested solver, this sets the accuracy ceiling: the convergence
/// tolerance is derived from it via [`kkt_tol`], so the two are linked.
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

    /// Cost `½‖R‖²` at `x`, used to scale [`STALL_REL_TOL`] to the problem.
    ///
    /// Deliberately not [`CostFunction::cost`]: this is solver setup rather than
    /// an optimizer evaluation, so it must not inflate the reported
    /// `n_residual_evals`. Falls back to `1.0` for a non-finite or zero cost, so
    /// the tolerance stays a usable absolute number.
    fn cost_scale(&self, x: &Col<f64>) -> f64 {
        let cost = 0.5 * self.residual_at(x).iter().map(|v| v * v).sum::<f64>();
        if cost.is_finite() && cost > 0.0 {
            cost
        } else {
            1.0
        }
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
    fn evaluate(&self, x: &Col<f64>) -> (Vec<GoodPrice>, f64, f64, ShopSnapshot) {
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

        // Raw residual targets unclipped τ (the solver's objective); the capped one
        // targets `clamp(τ)`, the game's own price rule, so a component pinned at a
        // bound it cannot pass contributes zero. Recover τ from `R`: the market
        // block is `R = r − τ`, the pure-state block `R = (σ − τ_state) / base_g`,
        // so `scale` converts a residual component back into the units of `x`.
        let res = self.residual_at(x);
        let (sum_sq, capped_sum_sq) =
            (0..res.nrows()).fold((0.0_f64, 0.0_f64), |(raw_acc, capped_acc), i| {
                let raw = res[i];
                let scale = if i < g { 1.0 } else { self.bases[(i - g) % g] };
                let tau = x[i] - raw * scale;
                let capped = (x[i] - tau.clamp(self.lower[i], self.upper[i])) / scale;
                (raw_acc + raw * raw, capped_acc + capped * capped)
            });
        let residual = sum_sq.sqrt();
        let capped_residual = capped_sum_sq.sqrt();

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

        (rows, residual, capped_residual, snapshot)
    }
}

impl CostFunction for PriceResidualJoint<'_> {
    type Param = Col<f64>;
    type Output = f64;
    type Error = Infallible;

    /// Half the squared residual norm, for Basin's cost-based stopping hooks.
    fn cost(&self, param: &Col<f64>) -> Result<f64, Infallible> {
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
        self.n_residual_evals.set(self.n_residual_evals.get() + 1);
        Ok(self.residual_at(param))
    }
}

impl Jacobian for PriceResidualJoint<'_> {
    type Jacobian = ArrowheadMat;

    /// Sparse Jacobian `∂R/∂x` at `param`.
    ///
    /// Market columns (`r_j`) use standard finite differences. Pure-state columns
    /// (`σ_{s,j}`) batch FD for the separable state block and analytical τ
    /// derivatives for market rows (see module docs).
    fn jacobian(&self, param: &Col<f64>) -> Result<ArrowheadMat, Infallible> {
        self.n_jacobian_evals.set(self.n_jacobian_evals.get() + 1);
        let g = self.n_goods();
        let s = self.n_states();

        let mut base_pop_buys = vec![0.0; s * g];
        let r0 = self.eval_residual(param, Some(&mut base_pop_buys));
        let mut jac = ArrowheadMat::zeros(g, s);

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
                for (s_idx, shop) in self.cache.shops.iter().enumerate() {
                    state_sum +=
                        shop.access * (base_pop_buys[s_idx * g + i] - shop.frozen_pop_buy[good]);
                }
                base_world_buy[i] = self.cache.frozen_buy[good] + world_pop_buy[good] + state_sum;
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

            for i in 0..r0.nrows() {
                let deriv = (x1[i] - r0[i]) / denom;
                if deriv.abs() > 1e-12 {
                    jac.set_entry(i, j, deriv);
                }
            }
        }

        let mut stepped_pop_buys = vec![0.0; s * g];

        // 2. Perturb pure-state prices σ_{s,j}
        for j in 0..g {
            let mut batched_stepped = param.clone();
            let mut denoms = vec![0.0; s];
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

                for (i, &good) in self.goods.iter().enumerate() {
                    let delta_buy = stepped_pop_buys[s_idx * g + i] - base_pop_buys[s_idx * g + i];
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
                            jac.set_entry(i, idx, deriv);
                        }
                    }
                }

                for i in 0..g {
                    let r_idx = g + s_idx * g + i;
                    let deriv = (batched_x1[r_idx] - r0[r_idx]) / denom;
                    if deriv.abs() > 1e-12 {
                        jac.set_entry(r_idx, idx, deriv);
                    }
                }
            }
        }

        Ok(jac)
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
    let goods = market_goods(&cache.base_prices);

    // Degenerate markets: nothing to price.
    if goods.is_empty() {
        return (
            SolveOutcome {
                goods: Vec::new(),
                residual: 0.0,
                capped_residual: 0.0,
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
                capped_residual: f64::INFINITY,
                status: SolveStatus::Failed,
                relative: Vec::new(),
                building_revenues: Vec::new(),
                stats: empty_stats(strategy),
            },
            None,
        );
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
    };

    let basin_iters = u64::from(opts.max_iters);
    // BCL §2 reflection: without it, coordinates that hit the box while others
    // still have large scaled KKT drive Coleman–Li `d² → ∞` and `SolverFailed`.
    // Stationarity is the convergence test; the stall counter only catches solves
    // that never get there, and reports itself as unsuccessful.
    // `cost = ½‖r‖²`, so `‖r‖ = sqrt(2·cost)`.
    let cost = problem.cost_scale(&x);
    let stall_tol = STALL_REL_TOL * cost;
    let grad_tol = kkt_tol(FD_STEP, (2.0 * cost).sqrt());
    let result = Executor::from_start(
        problem.clone(),
        Trf::new().with_reflection(true).with_tol_grad(grad_tol),
        x.clone(),
    )
    .max_iter(basin_iters)
    .terminate_on(NoImprovement::new(STALL_PATIENCE, stall_tol))
    .run();

    let outcome = match result {
        Ok(o) => o,
        Err(e) => match e {},
    };

    x.clone_from(outcome.param());
    let (rows, residual, capped_residual, snapshot) = problem.evaluate(&x);
    let building_revenues =
        crate::report::building_revenues_from_cache(cache, defs, &rows, Some(&snapshot));

    // Basin is the authority on whether it reached a constrained optimum. The raw
    // ‖R‖ stays large whenever unclipped τ lies outside the price box (capped /
    // disequilibrium prices; see docs/prices-equilibrium.md), so it is the capped
    // residual that says whether the answer matches the game's price rule.
    let status = match outcome.reason {
        TerminationReason::SolverFailed => SolveStatus::Failed,
        TerminationReason::SolverConverged => SolveStatus::Converged,
        // Backstop, not convergence: the solve stopped improving without reaching
        // first-order optimality.
        TerminationReason::NoImprovement => SolveStatus::Stalled,
        _ => SolveStatus::MaxIters,
    };

    (
        SolveOutcome {
            goods: rows,
            residual,
            capped_residual,
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
    )
}

#[cfg(test)]
mod tests {
    //! White-box tests for the joint [`ArrowheadMat`] path used by [`equilibrate_joint`].
    //!
    //! # Test layers
    //!
    //! 1. **Jacobian** — batched/analytical assembly matches independent FD columns.
    //! 2. **Gram oracles** — at a fixed iterate, Arrowhead `max_diagonal` and damped
    //!    `solve_spd` match dense `JᵀJ` Cholesky and Basin sparse Gram (ground truth).
    //! 3. **TRF trajectory** — [`TrfReplay`] re-implements Basin's outer loop (carried
    //!    `μ`, accept/reject, inner damping retries) and checks Arrowhead vs sparse
    //!    Newton steps at each iterate.
    //! 4. **Bound-constrained TRF** — some `x_i` sit at `lower_i`/`upper_i` while
    //!    others still violate scaled KKT. Default θ-step-back collapses `dist_i`
    //!    until `d²` overflows; [`Trf::with_reflection`](basin::Trf::with_reflection)
    //!    keeps the face finite. See [`trf_with_reflection_survives_near_box`].
    //! 5. **End-to-end** — [`toy_joint_equilibrate_finishes`]: Basin finishes without
    //!    `SolverFailed`. `SolveStatus::Converged` is only for ‖R‖ < eps (cleared
    //!    within the box). Face-active stops with leftover residual are expected when
    //!    unclipped τ is outside `[1±ρ]` — design, not a solver miss.
    //!
    //! Property tests randomize damping scale on the toy fixture while keeping the
    //! iterate fixed at the TRF start (fast, deterministic physics).

    use super::*;
    use crate::shop_cache::ShopCache;
    use crate::{SolveStatus, SolveStrategy, World};
    use basin::{
        AddDiagonalVectorInPlace, Dot, Executor, GramMatrix, LinearSolveSpd, MatTransposeVec,
        MaxDiagonal, NegInPlace, NormSquared, ScaledAdd, Trf,
    };
    use faer::linalg::matmul::matmul;
    use faer::linalg::solvers::{Llt, Solve};
    use faer::sparse::{SparseColMat, Triplet};
    use faer::{Accum, Col, Mat, Par, Side};
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn toy_defs_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vic3-defs/tests/fixtures/toy_economy")
    }

    fn toy_save_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vic3-load/tests/fixtures/toy_economy.txt")
    }

    /// Branch–Coleman–Li `(d²_i, c_i)` for one coordinate (cases i–iv).
    fn cl_scaling_pair(x: f64, grad: f64, lower: f64, upper: f64) -> (f64, f64) {
        if grad < 0.0 {
            if upper.is_finite() {
                let abs_v = upper - x;
                (1.0 / abs_v, (-grad) / abs_v)
            } else {
                (1.0, 0.0)
            }
        } else if lower.is_finite() {
            let abs_v = x - lower;
            (1.0 / abs_v, grad / abs_v)
        } else {
            (1.0, 0.0)
        }
    }

    struct ToyJointSetup {
        defs: vic3_defs::GameDefs,
        cache: Arc<ShopCache>,
        goods: Vec<GoodId>,
        bases: Vec<f64>,
        res_evals: Cell<u64>,
        jac_evals: Cell<u64>,
        x: Col<f64>,
        lower: Col<f64>,
        upper: Col<f64>,
    }

    impl ToyJointSetup {
        fn n(&self) -> usize {
            self.x.nrows()
        }

        fn problem(&self) -> PriceResidualJoint<'_> {
            PriceResidualJoint {
                defs: &self.defs,
                cache: self.cache.clone(),
                bases: &self.bases,
                goods: &self.goods,
                upper: self.upper.clone(),
                lower: self.lower.clone(),
                price_range: self.defs.price_range.max(0.0),
                n_residual_evals: &self.res_evals,
                n_jacobian_evals: &self.jac_evals,
            }
        }
    }

    /// Same TRF start as [`equilibrate_joint`] (relative market + absolute σ at base).
    fn toy_joint_fixture() -> ToyJointSetup {
        let defs = vic3_defs::load_from_path(toy_defs_root()).expect("toy defs");
        let save =
            vic3_load::load_path(toy_save_path(), vic3_load::empty_tokens()).expect("toy save");
        let world = World::from_save(&save, &defs);

        let cache = Arc::new(ShopCache::from_world(&world, &defs));
        let goods = market_goods(&cache.base_prices);
        let bases: Vec<f64> = goods.iter().map(|&id| cache.base_prices[id]).collect();
        let g = goods.len();
        let s = cache.shops.len();
        let n = g + s * g;
        let price_range = defs.price_range.max(0.0);

        let mut lower = Col::zeros(n);
        let mut upper = Col::zeros(n);
        for (i, _) in bases.iter().enumerate() {
            lower[i] = 1.0 - price_range;
            upper[i] = 1.0 + price_range;
        }
        for s_idx in 0..s {
            for (i, &base) in bases.iter().enumerate() {
                let idx = g + s_idx * g + i;
                lower[idx] = base * (1.0 - price_range);
                upper[idx] = base * (1.0 + price_range);
            }
        }

        let mut x = Col::zeros(n);
        for (i, _) in bases.iter().enumerate() {
            x[i] = 1.0;
        }
        for s_idx in 0..s {
            for (i, &base) in bases.iter().enumerate() {
                x[g + s_idx * g + i] = base;
            }
        }

        ToyJointSetup {
            defs,
            cache,
            goods,
            bases,
            res_evals: Cell::new(0),
            jac_evals: Cell::new(0),
            x,
            lower,
            upper,
        }
    }

    fn dense_jtj(j: &ArrowheadMat) -> Mat<f64> {
        let jd = j.to_dense();
        let n = j.ncols();
        let mut jtj = Mat::<f64>::zeros(n, n);
        matmul(
            jtj.as_mut(),
            Accum::Replace,
            jd.transpose(),
            jd.as_ref(),
            1.0,
            Par::Seq,
        );
        jtj
    }

    fn dense_gram_solve(jtj: &Mat<f64>, rhs: &Col<f64>, damping: &Col<f64>) -> Col<f64> {
        let n = rhs.nrows();
        let mut a = jtj.clone();
        for i in 0..n {
            a[(i, i)] += damping[i];
        }
        let llt = Llt::new(a.as_ref(), faer::Side::Lower).expect("dense Gram + damp is SPD");
        let mut x = rhs.clone();
        llt.solve_in_place(x.as_mut());
        x
    }

    /// TRF damping `diag(c) + μ·diag(d²)` with `μ₀ = τ·max diag(JᵀJ + c)`.
    fn trf_damping(
        j: &ArrowheadMat,
        x: &Col<f64>,
        grad: &Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
        tau: f64,
    ) -> Col<f64> {
        let n = j.ncols();
        let mut c_diag = Col::zeros(n);
        let mut d_sq = Col::zeros(n);
        for i in 0..n {
            let (d, c) = cl_scaling_pair(x[i], grad[i], lower[i], upper[i]);
            d_sq[i] = d;
            c_diag[i] = c;
        }
        let mut gram = j.gram();
        gram.add_diagonal_vector_in_place(&c_diag);
        let max_diag = gram.max_diagonal().max(1.0);
        let mu = tau * max_diag;
        Col::from_fn(n, |i| c_diag[i] + mu * d_sq[i])
    }

    fn residual_norm(problem: &PriceResidualJoint<'_>, x: &Col<f64>) -> f64 {
        let r = problem.residual_at(x);
        r.iter().map(|v| v * v).sum::<f64>().sqrt()
    }

    fn sparse_jacobian_from_dense(j: &ArrowheadMat) -> SparseColMat<usize, f64> {
        let jd = j.to_dense();
        let n = j.ncols();
        let mut triplets = Vec::new();
        for col in 0..n {
            for row in 0..n {
                let v = jd[(row, col)];
                if v != 0.0 {
                    triplets.push(Triplet::new(row, col, v));
                }
            }
        }
        SparseColMat::try_new_from_triplets(n, n, &triplets).expect("sparse J from dense")
    }

    /// Run Basin TRF for `steps` iterations from `x0`; returns final `x` and stop reason.
    fn trf_param_after_steps(
        problem: &PriceResidualJoint<'_>,
        x0: &Col<f64>,
        steps: u64,
    ) -> (Col<f64>, TerminationReason) {
        let outcome = Executor::from_start(
            problem.clone(),
            Trf::new().with_reflection(true),
            x0.clone(),
        )
        .max_iter(steps)
        .run()
        .expect("executor");
        (outcome.param().clone(), outcome.reason)
    }

    /// Arrowhead damped Gram solve must match dense and sparse oracles at iterate `x`.
    ///
    /// Uses TRF-style damping `diag(c) + μ·diag(d²)` with default `τ = 1e-3`.
    fn assert_gram_solve_matches_oracles(
        problem: &PriceResidualJoint<'_>,
        x: &Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
        label: &str,
    ) {
        let j = problem.jacobian(x).unwrap();
        let r = problem.residual_at(x);
        let grad = j.mat_transpose_vec(&r);
        let neg_g = Col::from_fn(x.nrows(), |i| -grad[i]);
        let damping = trf_damping(&j, x, &grad, lower, upper, 1e-3);
        let n = x.nrows();

        let mut c_diag = Col::zeros(n);
        for i in 0..n {
            c_diag[i] = cl_scaling_pair(x[i], grad[i], lower[i], upper[i]).1;
        }
        let mut gram = j.gram();
        gram.add_diagonal_vector_in_place(&c_diag);
        let structured_max = gram.max_diagonal();
        let mut dense = dense_jtj(&j);
        for i in 0..n {
            dense[(i, i)] += c_diag[i];
        }
        let dense_max = (0..n)
            .map(|i| dense[(i, i)])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (structured_max - dense_max).abs() < 1e-8,
            "{label}: max_diagonal structured={structured_max} dense={dense_max}",
        );

        let mut gram_damped = j.gram();
        gram_damped.add_diagonal_vector_in_place(&damping);
        let h_arrow = gram_damped
            .solve_spd(&neg_g)
            .unwrap_or_else(|e| panic!("{label}: Arrowhead solve_spd failed: {e:?}"));
        let h_dense = dense_gram_solve(&dense_jtj(&j), &neg_g, &damping);

        let sparse_j = sparse_jacobian_from_dense(&j);
        let mut sparse_gram = sparse_j.gram();
        sparse_gram.add_diagonal_vector_in_place(&damping);
        let h_sparse = sparse_gram
            .solve_spd(&neg_g)
            .unwrap_or_else(|e| panic!("{label}: sparse Gram solve_spd failed: {e:?}"));

        for i in 0..n {
            assert!(
                (h_arrow[i] - h_dense[i]).abs() < 1e-4,
                "{label}: arrow vs dense at {i}: {} vs {}",
                h_arrow[i],
                h_dense[i],
            );
            assert!(
                (h_arrow[i] - h_sparse[i]).abs() < 1e-4,
                "{label}: arrow vs sparse at {i}: {} vs {}",
                h_arrow[i],
                h_sparse[i],
            );
        }
    }

    /// Same oracle checks with an explicit diagonal damping vector (for property tests).
    fn assert_damped_solve_matches_oracles(
        problem: &PriceResidualJoint<'_>,
        x: &Col<f64>,
        damping: &Col<f64>,
        label: &str,
    ) {
        let j = problem.jacobian(x).unwrap();
        let r = problem.residual_at(x);
        let grad = j.mat_transpose_vec(&r);
        let neg_g = Col::from_fn(x.nrows(), |i| -grad[i]);
        let n = x.nrows();

        let mut gram_damped = j.gram();
        gram_damped.add_diagonal_vector_in_place(damping);
        let h_arrow = gram_damped
            .solve_spd(&neg_g)
            .unwrap_or_else(|e| panic!("{label}: Arrowhead solve_spd failed: {e:?}"));
        let h_dense = dense_gram_solve(&dense_jtj(&j), &neg_g, damping);

        let sparse_j = sparse_jacobian_from_dense(&j);
        let mut sparse_gram = sparse_j.gram();
        sparse_gram.add_diagonal_vector_in_place(damping);
        let h_sparse = sparse_gram
            .solve_spd(&neg_g)
            .unwrap_or_else(|e| panic!("{label}: sparse Gram solve_spd failed: {e:?}"));

        for i in 0..n {
            assert!(
                (h_arrow[i] - h_dense[i]).abs() < 1e-4,
                "{label}: arrow vs dense at {i}",
            );
            assert!(
                (h_arrow[i] - h_sparse[i]).abs() < 1e-4,
                "{label}: arrow vs sparse at {i}",
            );
        }
    }

    /// Strict-interior projection (mirrors Basin `BoxAffineScaling::project_strictly_inside`).
    fn project_strictly_inside_col(
        x: &mut Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
        rstep: f64,
    ) {
        let n = x.nrows();
        for i in 0..n {
            let lo_inner = if lower[i].is_finite() {
                lower[i] + rstep * lower[i].abs().max(1.0)
            } else {
                f64::NEG_INFINITY
            };
            let hi_inner = if upper[i].is_finite() {
                upper[i] - rstep * upper[i].abs().max(1.0)
            } else {
                f64::INFINITY
            };
            let v = x[i];
            x[i] = if v < lo_inner {
                lo_inner
            } else if v > hi_inner {
                hi_inner
            } else {
                v
            };
        }
    }

    /// Largest τ with `x + τ·step` in the box (mirrors Basin `max_feasible_step`).
    fn max_feasible_step_col(
        x: &Col<f64>,
        step: &Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
    ) -> f64 {
        let n = x.nrows();
        let mut tau = f64::INFINITY;
        for i in 0..n {
            let t = if step[i] > 0.0 {
                if upper[i].is_finite() {
                    (upper[i] - x[i]) / step[i]
                } else {
                    f64::INFINITY
                }
            } else if step[i] < 0.0 {
                if lower[i].is_finite() {
                    (lower[i] - x[i]) / step[i]
                } else {
                    f64::INFINITY
                }
            } else {
                f64::INFINITY
            };
            tau = tau.min(t);
        }
        tau
    }

    fn weighted_norm_squared_col(v: &Col<f64>, weights: &Col<f64>) -> f64 {
        v.iter().zip(weights.iter()).map(|(a, w)| a * a * w).sum()
    }

    fn cl_scaling(
        x: &Col<f64>,
        grad: &Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
    ) -> (Col<f64>, Col<f64>) {
        let n = x.nrows();
        let mut d_sq = Col::zeros(n);
        let mut c_diag = Col::zeros(n);
        for i in 0..n {
            let (d, c) = cl_scaling_pair(x[i], grad[i], lower[i], upper[i]);
            d_sq[i] = d;
            c_diag[i] = c;
        }
        (d_sq, c_diag)
    }

    fn trf_init_mu(j: &ArrowheadMat, c_diag: &Col<f64>, tau: f64) -> f64 {
        let mut gram = j.gram();
        gram.add_diagonal_vector_in_place(c_diag);
        tau * gram.max_diagonal().max(1.0)
    }

    fn trf_init_mu_sparse(j: &ArrowheadMat, c_diag: &Col<f64>, tau: f64) -> f64 {
        let sparse_j = sparse_jacobian_from_dense(j);
        let mut gram = sparse_j.gram();
        gram.add_diagonal_vector_in_place(c_diag);
        tau * gram.max_diagonal().max(1.0)
    }

    fn damping_from_cl(mu: f64, c_diag: &Col<f64>, d_sq: &Col<f64>) -> Col<f64> {
        let n = c_diag.nrows();
        Col::from_fn(n, |i| c_diag[i] + mu * d_sq[i])
    }

    /// Result of one damped Newton solve: Arrowhead vs sparse oracle paths.
    #[derive(Debug, Clone)]
    struct DampedNewtonCompare {
        h_arrow: Option<Col<f64>>,
        h_sparse: Option<Col<f64>>,
    }

    impl DampedNewtonCompare {
        fn max_abs_diff(&self) -> f64 {
            match (&self.h_arrow, &self.h_sparse) {
                (Some(a), Some(s)) => a
                    .iter()
                    .zip(s.iter())
                    .map(|(x, y)| (x - y).abs())
                    .fold(0.0, f64::max),
                _ => f64::NAN,
            }
        }
    }

    fn damped_newton_compare(
        j: &ArrowheadMat,
        neg_g: &Col<f64>,
        damping: &Col<f64>,
    ) -> DampedNewtonCompare {
        let mut gram = j.gram();
        gram.add_diagonal_vector_in_place(damping);
        let h_arrow = gram.solve_spd(neg_g).ok();

        let sparse_j = sparse_jacobian_from_dense(j);
        let mut sparse_gram = sparse_j.gram();
        sparse_gram.add_diagonal_vector_in_place(damping);
        let h_sparse = sparse_gram.solve_spd(neg_g).ok();

        DampedNewtonCompare { h_arrow, h_sparse }
    }

    /// Snapshot of one TRF outer iteration (for regression tests on carried `μ`).
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct TrfIterSnapshot {
        iter: u32,
        mu_in: f64,
        /// `μ` at which the damped Newton solve succeeded (may exceed `mu_in` after inner retries).
        mu_solved: f64,
        mu_out: f64,
        inner_attempts: u32,
        accepted: bool,
        residual: f64,
        newton: DampedNewtonCompare,
    }

    /// Minimal TRF outer-loop replay mirroring Basin [`Trf`] (τ, θ, Nielsen `μ`).
    struct TrfReplay {
        tau: f64,
        theta: f64,
        max_inner_attempts: u32,
        rstep: f64,
    }

    impl Default for TrfReplay {
        fn default() -> Self {
            Self {
                tau: 1e-3,
                theta: 0.99995,
                max_inner_attempts: 50,
                rstep: 1e-10,
            }
        }
    }

    impl TrfReplay {
        fn init_state(
            &self,
            problem: &PriceResidualJoint<'_>,
            x0: &Col<f64>,
            lower: &Col<f64>,
            upper: &Col<f64>,
        ) -> (Col<f64>, f64, f64, f64) {
            let mut x = x0.clone();
            project_strictly_inside_col(&mut x, lower, upper, self.rstep);
            let r = problem.residual_at(&x);
            let cost = 0.5 * r.norm_squared();
            let j = problem.jacobian(&x).unwrap();
            let grad = j.mat_transpose_vec(&r);
            let (_, c_diag) = cl_scaling(&x, &grad, lower, upper);
            let mu = trf_init_mu(&j, &c_diag, self.tau);
            (x, mu, 2.0, cost)
        }

        /// Run up to `max_iters` outer steps; returns per-iter snapshots and optional fail iter.
        fn run(
            &self,
            problem: &PriceResidualJoint<'_>,
            x0: &Col<f64>,
            lower: &Col<f64>,
            upper: &Col<f64>,
            max_iters: u32,
        ) -> (Col<f64>, Vec<TrfIterSnapshot>, Option<u32>) {
            let (mut x, mut mu, mut nu, mut cost) = self.init_state(problem, x0, lower, upper);
            let mut snaps = Vec::new();

            for iter in 1..=max_iters {
                let r = problem.residual_at(&x);
                let j = problem.jacobian(&x).unwrap();
                let grad = j.mat_transpose_vec(&r);
                let (d_sq, c_diag) = cl_scaling(&x, &grad, lower, upper);
                let mut neg_g = grad.clone();
                neg_g.neg_in_place();

                let mu_in = mu;
                let mut inner_attempts = 0u32;
                let (h, mu_solved, newton_at_solve) = loop {
                    inner_attempts += 1;
                    let damping = damping_from_cl(mu, &c_diag, &d_sq);
                    let cmp = damped_newton_compare(&j, &neg_g, &damping);
                    if let Some(h_arrow) = &cmp.h_arrow {
                        break (h_arrow.clone(), mu, cmp);
                    }
                    if inner_attempts >= self.max_inner_attempts || !mu.is_finite() {
                        snaps.push(TrfIterSnapshot {
                            iter,
                            mu_in,
                            mu_solved: mu,
                            mu_out: mu,
                            inner_attempts,
                            accepted: false,
                            residual: residual_norm(problem, &x),
                            newton: cmp,
                        });
                        return (x, snaps, Some(iter));
                    }
                    mu *= nu;
                    nu *= 2.0;
                };
                mu = mu_solved;

                let tau_max = max_feasible_step_col(&x, &h, lower, upper);
                let alpha = if tau_max >= 1.0 {
                    1.0
                } else {
                    self.theta * tau_max
                };

                let mut x_trial = x.clone();
                x_trial.scaled_add(alpha, &h);
                let r_trial = problem.residual_at(&x_trial);
                let f_trial = 0.5 * r_trial.norm_squared();

                let h_t_g = h.dot(&grad);
                let dh_norm_sq = weighted_norm_squared_col(&h, &d_sq);
                let predicted = -alpha * (1.0 - 0.5 * alpha) * h_t_g
                    + 0.5 * alpha * alpha * mu_solved * dh_norm_sq;
                let half_s_t_c_s = 0.5 * alpha * alpha * weighted_norm_squared_col(&h, &c_diag);
                let actual = cost - f_trial - half_s_t_c_s;
                let rho = if predicted > 0.0 {
                    actual / predicted
                } else {
                    0.0
                };

                let accepted = rho > 0.0;

                if accepted {
                    x = x_trial;
                    cost = f_trial;
                    let factor = (1.0 - (2.0 * rho - 1.0).powi(3)).max(1.0 / 3.0);
                    mu *= factor;
                    nu = 2.0;
                } else {
                    mu *= nu;
                    nu *= 2.0;
                }

                snaps.push(TrfIterSnapshot {
                    iter,
                    mu_in,
                    mu_solved,
                    mu_out: mu,
                    inner_attempts,
                    accepted,
                    residual: residual_norm(problem, &x),
                    newton: newton_at_solve,
                });
            }

            (x, snaps, None)
        }
    }

    fn assert_newton_matches_sparse(snapshot: &TrfIterSnapshot) {
        let diff = snapshot.newton.max_abs_diff();
        assert!(
            snapshot.newton.h_arrow.is_some() && snapshot.newton.h_sparse.is_some(),
            "iter {}: arrow_ok={} sparse_ok={} (mu_in={})",
            snapshot.iter,
            snapshot.newton.h_arrow.is_some(),
            snapshot.newton.h_sparse.is_some(),
            snapshot.mu_in,
        );
        assert!(
            diff < 1e-4,
            "iter {}: arrow vs sparse Newton step max diff {diff} (mu_in={})",
            snapshot.iter,
            snapshot.mu_in,
        );
    }

    /// Carried `μ₀` from Arrowhead `max_diagonal` matches the sparse-Gram oracle.
    #[test]
    fn toy_trf_replay_init_mu_arrow_matches_sparse() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let r = problem.residual_at(&setup.x);
        let j = problem.jacobian(&setup.x).unwrap();
        let grad = j.mat_transpose_vec(&r);
        let (_, c_diag) = cl_scaling(&setup.x, &grad, &setup.lower, &setup.upper);
        let mu_arrow = trf_init_mu(&j, &c_diag, 1e-3);
        let mu_sparse = trf_init_mu_sparse(&j, &c_diag, 1e-3);
        assert!(
            (mu_arrow - mu_sparse).abs() < 1e-8 * mu_sparse.abs().max(1.0),
            "mu_arrow={mu_arrow} mu_sparse={mu_sparse}",
        );
    }

    /// With **carried** `μ`, each Newton step matches sparse through five outer iterations.
    #[test]
    fn toy_trf_replay_newton_steps_match_sparse_through_five() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let (_, snaps, fail) =
            TrfReplay::default().run(&problem, &setup.x, &setup.lower, &setup.upper, 5);
        assert!(fail.is_none(), "SolverFailed at iter {fail:?}");
        assert_eq!(snaps.len(), 5);
        for snap in &snaps {
            assert_newton_matches_sparse(snap);
        }
    }

    /// Reconstruct TRF state at the start of outer iteration `target_iter` (1-based).
    fn trf_replay_state_at_iter(target_iter: u32) -> (ToyJointSetup, Col<f64>, f64) {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let prev = target_iter.saturating_sub(1);
        let (x, snaps, fail) =
            TrfReplay::default().run(&problem, &setup.x, &setup.lower, &setup.upper, prev);
        assert!(
            fail.is_none(),
            "replay failed before iter {target_iter}: {fail:?}",
        );
        let mu = if prev == 0 {
            let r = problem.residual_at(&setup.x);
            let j = problem.jacobian(&setup.x).unwrap();
            let grad = j.mat_transpose_vec(&r);
            let (_, c_diag) = cl_scaling(&setup.x, &grad, &setup.lower, &setup.upper);
            trf_init_mu(&j, &c_diag, 1e-3)
        } else {
            snaps.last().expect("prior iter snapshot").mu_out
        };
        (setup, x, mu)
    }

    fn self_adjoint_eigenvalues_oracle(a: &Mat<f64>) -> Option<Vec<f64>> {
        a.self_adjoint_eigenvalues(Side::Lower).ok()
    }

    /// `None` once the spectrum stops being meaningful: an overflowing Coleman–Li `d²`
    /// puts a non-finite entry on the diagonal, and backends differ on whether that is
    /// reported as a decomposition error or as NaN eigenvalues. `f64::min` skips NaN, so
    /// the fold has to be guarded rather than trusted.
    fn min_max_eigenvalues(a: &Mat<f64>) -> Option<(f64, f64)> {
        let evals = self_adjoint_eigenvalues_oracle(a)?;
        if evals.is_empty() || evals.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let min = evals.iter().copied().fold(f64::INFINITY, f64::min);
        let max = evals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some((min, max))
    }

    /// Damped Gram `JᵀJ + diag(c + μ d²)` at a TRF iterate. Pins non-finite diagonal
    /// when a capped price has `(bound − x)` in the denominator of Coleman–Li scaling.
    struct DampedGramSnapshot {
        mu: f64,
        min_damping: f64,
        min_eigenvalue: Option<f64>,
        max_eigenvalue: Option<f64>,
        dense_cholesky_ok: bool,
    }

    impl DampedGramSnapshot {
        fn condition_number(&self) -> Option<f64> {
            match (self.min_eigenvalue, self.max_eigenvalue) {
                (Some(min), Some(max)) if min > 0.0 => Some(max / min),
                _ => None,
            }
        }
    }

    fn damped_gram_snapshot_for_x(
        setup: &ToyJointSetup,
        x: &Col<f64>,
        mu: f64,
    ) -> DampedGramSnapshot {
        let problem = setup.problem();
        let j = problem.jacobian(x).unwrap();
        let r = problem.residual_at(x);
        let grad = j.mat_transpose_vec(&r);
        let (d_sq, c_diag) = cl_scaling(x, &grad, &setup.lower, &setup.upper);
        let damping = damping_from_cl(mu, &c_diag, &d_sq);
        let mut dense = dense_jtj(&j);
        for i in 0..x.nrows() {
            dense[(i, i)] += damping[i];
        }
        let (min_eigenvalue, max_eigenvalue) = match min_max_eigenvalues(&dense) {
            Some((min, max)) => (Some(min), Some(max)),
            None => (None, None),
        };
        DampedGramSnapshot {
            mu,
            min_damping: damping.iter().copied().fold(f64::INFINITY, f64::min),
            min_eigenvalue,
            max_eigenvalue,
            dense_cholesky_ok: Llt::new(dense.as_ref(), Side::Lower).is_ok(),
        }
    }

    fn damped_gram_snapshot(target_iter: u32) -> DampedGramSnapshot {
        let (setup, x, mu) = trf_replay_state_at_iter(target_iter);
        damped_gram_snapshot_for_x(&setup, &x, mu)
    }

    /// Same iterate as `target_iter` but override carried `μ` (isolates μ vs `x` effects).
    fn damped_gram_snapshot_at_x_with_mu(target_iter: u32, mu: f64) -> DampedGramSnapshot {
        let (setup, x, _) = trf_replay_state_at_iter(target_iter);
        damped_gram_snapshot_for_x(&setup, &x, mu)
    }

    /// Set `x[idx] = upper[idx] − gap` (synthetic near-upper-bound iterate).
    fn x_near_upper_bound(x: &Col<f64>, upper: &Col<f64>, idx: usize, gap: f64) -> Col<f64> {
        let mut out = x.clone();
        out[idx] = upper[idx] - gap;
        out
    }

    /// Set `x[idx] = lower[idx] + gap` (synthetic near-lower-bound iterate).
    fn x_near_lower_bound(x: &Col<f64>, lower: &Col<f64>, idx: usize, gap: f64) -> Col<f64> {
        let mut out = x.clone();
        out[idx] = lower[idx] + gap;
        out
    }

    fn relative_gap_to_upper(x: f64, upper: f64) -> f64 {
        (upper - x) / upper.abs().max(1.0)
    }

    fn relative_gap_to_lower(x: f64, lower: f64) -> f64 {
        (x - lower) / lower.abs().max(1.0)
    }

    /// Coleman–Li active bound for coordinate `idx` (cases i–ii in [`cl_scaling_pair`]).
    enum ActiveBound {
        Upper,
        Lower,
    }

    fn active_bound_at(grad_i: f64) -> ActiveBound {
        if grad_i < 0.0 {
            ActiveBound::Upper
        } else {
            ActiveBound::Lower
        }
    }

    fn relative_gap_to_active_bound(x: f64, lower: f64, upper: f64, grad_i: f64) -> f64 {
        match active_bound_at(grad_i) {
            ActiveBound::Upper => relative_gap_to_upper(x, upper),
            ActiveBound::Lower => relative_gap_to_lower(x, lower),
        }
    }

    fn x_near_active_bound(
        x: &Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
        idx: usize,
        gap: f64,
        grad_i: f64,
    ) -> Col<f64> {
        match active_bound_at(grad_i) {
            ActiveBound::Upper => x_near_upper_bound(x, upper, idx, gap),
            ActiveBound::Lower => x_near_lower_bound(x, lower, idx, gap),
        }
    }

    fn active_bound_gap(
        x: &Col<f64>,
        lower: &Col<f64>,
        upper: &Col<f64>,
        idx: usize,
        grad_i: f64,
    ) -> f64 {
        match active_bound_at(grad_i) {
            ActiveBound::Upper => upper[idx] - x[idx],
            ActiveBound::Lower => x[idx] - lower[idx],
        }
    }

    fn non_finite_damping_indices(damping: &Col<f64>) -> Vec<usize> {
        damping
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.is_finite())
            .map(|(i, _)| i)
            .collect()
    }

    /// Smallest singular value of `J` at a TRF iterate (rank diagnostic).
    fn jacobian_min_singular_value(target_iter: u32) -> f64 {
        let (setup, x, _) = trf_replay_state_at_iter(target_iter);
        let j = setup.problem().jacobian(&x).unwrap().to_dense();
        let svd = j.thin_svd().expect("thin SVD");
        let s = svd.S().column_vector();
        s.iter().copied().fold(f64::INFINITY, f64::min)
    }

    /// Undamped `JᵀJ` smallest eigenvalue via `σ_min(J)²`.
    fn gram_min_eigenvalue(target_iter: u32) -> f64 {
        let smin = jacobian_min_singular_value(target_iter);
        smin * smin
    }

    /// Joint lifting does not make `J` rank-deficient on the toy fixture at iter 1 or 6.
    #[test]
    fn toy_jacobian_full_rank_at_iter_one_and_six() {
        let s1 = jacobian_min_singular_value(1);
        let s6 = jacobian_min_singular_value(6);
        assert!(
            s1 > 1e-8,
            "σ_min(J) at iter 1 should be positive (got {s1})",
        );
        assert!(
            s6 > 1e-8,
            "σ_min(J) at iter 6 should be positive (got {s6})",
        );
    }

    /// Undamped `JᵀJ` stays SPD at iter six — failure is in `diag(c + μ d²)`, not in `J`.
    #[test]
    fn toy_gram_without_damping_still_spd_at_iter_six() {
        let g1 = gram_min_eigenvalue(1);
        let g6 = gram_min_eigenvalue(6);
        assert!(g1 > 1e-10, "min λ(JᵀJ) at iter 1: {g1}");
        assert!(
            g6 > 1e-10,
            "min λ(JᵀJ) at iter 6 should stay positive (got {g6})",
        );
    }

    /// Damped Gram is clearly SPD at TRF iter-one start.
    #[test]
    fn toy_damped_gram_spd_at_iter_one() {
        let s = damped_gram_snapshot(1);
        assert!(
            s.dense_cholesky_ok,
            "dense Cholesky should succeed at iter 1"
        );
        let min_eig = s.min_eigenvalue.expect("EVD should converge at iter 1");
        assert!(
            min_eig > 1e-6,
            "min eigenvalue at iter 1: {min_eig} (μ={})",
            s.mu,
        );
        let cond = s.condition_number().expect("condition at iter 1");
        assert!(cond < 1e8, "condition at iter 1 too large: {cond}");
    }

    /// Carried `μ` drops sharply after five TRF accepts (Nielsen), stripping regularization.
    #[test]
    fn toy_trf_carried_mu_collapses_before_iter_six() {
        let s1 = damped_gram_snapshot(1);
        let s6 = damped_gram_snapshot(6);
        assert!(
            s6.mu < s1.mu / 50.0,
            "μ should collapse: iter1={} iter6={}",
            s1.mu,
            s6.mu,
        );
        assert!(
            s6.min_damping < s1.min_damping / 50.0,
            "min damping: iter1={} iter6={}",
            s1.min_damping,
            s6.min_damping,
        );
    }

    /// At iter-six start, damped Gram fails on a capped price; `JᵀJ` without damping is still SPD.
    #[test]
    fn toy_damped_gram_singular_at_iter_six() {
        let s = damped_gram_snapshot(6);
        assert!(
            !s.dense_cholesky_ok,
            "dense Cholesky should fail at iter 6 (μ={})",
            s.mu,
        );
        // EVD may not converge on the singular matrix; Cholesky failure is the oracle.
        // `JᵀJ` is PSD, so λ_min of the damped Gram is at least the smallest damping
        // entry and an absolute bound on it says nothing; only λ_min/λ_max does.
        if let (Some(min), Some(max)) = (s.min_eigenvalue, s.max_eigenvalue) {
            assert!(
                min < 1e-10 * max,
                "expected min λ negligible against max λ (min={min}, max={max})",
            );
        }
    }

    /// Restoring iter-one `μ` at iter-six `x` still fails: `(upper_i − x_i)` is already too small.
    #[test]
    fn toy_damped_gram_iter_six_x_stays_ill_conditioned_with_restored_mu() {
        let s1 = damped_gram_snapshot(1);
        let s6 = damped_gram_snapshot(6);
        let s6_mu1 = damped_gram_snapshot_at_x_with_mu(6, s1.mu);
        assert!(!s6.dense_cholesky_ok);
        assert!(
            !s6_mu1.dense_cholesky_ok,
            "iter-6 x with iter-1 μ still fails (μ={} vs carried μ={})",
            s6_mu1.mu, s6.mu,
        );
    }

    /// Coleman–Li `d² = 1/(upper − x)` diverges as `x` approaches a finite upper bound.
    #[test]
    fn cl_scaling_pair_diverges_as_x_approaches_upper_bound() {
        let upper = 1.25;
        let lower = 0.75;
        let grad = -1.0;
        let (d_far, c_far) = cl_scaling_pair(upper - 1e-6, grad, lower, upper);
        let (d_near, c_near) = cl_scaling_pair(upper - 1e-18, grad, lower, upper);
        assert!(
            d_near > d_far * 1e6,
            "d² should grow sharply near the upper face"
        );
        assert!(
            c_near > c_far * 1e6,
            "c should grow sharply near the upper face"
        );
        let mu = 1.3e-5;
        let damping = c_near + mu * d_near;
        assert!(
            !damping.is_finite(),
            "carried-μ scale damping should overflow when x is within ~1e-18 of upper",
        );
    }

    /// Parking one coordinate at the active bound ± gap with carried `μ` yields non-finite damping.
    #[test]
    fn toy_synthetic_near_bound_damping_non_finite() {
        let setup = toy_joint_fixture();
        let (_, x6, mu6) = trf_replay_state_at_iter(6);
        let problem = setup.problem();
        let grad6 = problem
            .jacobian(&x6)
            .unwrap()
            .mat_transpose_vec(&problem.residual_at(&x6));
        let (d_sq6, c6) = cl_scaling(&x6, &grad6, &setup.lower, &setup.upper);
        let damping6 = damping_from_cl(mu6, &c6, &d_sq6);
        let bad = non_finite_damping_indices(&damping6);
        assert!(
            !bad.is_empty(),
            "iter-six oracle should have a bad damping index"
        );
        let idx = bad[0];
        let gap = active_bound_gap(&x6, &setup.lower, &setup.upper, idx, grad6[idx]);
        assert!(
            relative_gap_to_active_bound(x6[idx], setup.lower[idx], setup.upper[idx], grad6[idx],)
                < 1e-6,
            "iter-six bad coord {idx} should be within 1e-6 (rel) of its active bound",
        );

        // Rebuild from `x6` (not the TRF start) so Coleman–Li active-bound case
        // matches; gap may be 0 when the iterate is already on the face.
        let x_syn = x_near_active_bound(&x6, &setup.lower, &setup.upper, idx, gap, grad6[idx]);
        let snap_syn = damped_gram_snapshot_for_x(&setup, &x_syn, mu6);
        // Prefer an explicit non-finite check: `f64::min` ignores NaNs, so
        // `min_damping` can look finite while some `c + μ d²` entries are NaN/Inf.
        let problem = setup.problem();
        let grad_syn = problem
            .jacobian(&x_syn)
            .unwrap()
            .mat_transpose_vec(&problem.residual_at(&x_syn));
        let (d_syn, c_syn) = cl_scaling(&x_syn, &grad_syn, &setup.lower, &setup.upper);
        let damp_syn = damping_from_cl(mu6, &c_syn, &d_syn);
        assert!(
            !non_finite_damping_indices(&damp_syn).is_empty() || !snap_syn.min_damping.is_finite(),
            "synthetic x[idx={idx}] at slack {gap:.3e} should match iter-six non-finite damping \
             (min_damping={}, bad={:?})",
            snap_syn.min_damping,
            non_finite_damping_indices(&damp_syn),
        );
    }

    /// Synthetic iterate with `s_i` matching iter six: damped Gram Cholesky fails.
    #[test]
    fn toy_synthetic_near_bound_damped_gram_cholesky_fails() {
        let setup = toy_joint_fixture();
        let (_, x6, mu6) = trf_replay_state_at_iter(6);
        let problem = setup.problem();
        let grad6 = problem
            .jacobian(&x6)
            .unwrap()
            .mat_transpose_vec(&problem.residual_at(&x6));
        let (d_sq6, c6) = cl_scaling(&x6, &grad6, &setup.lower, &setup.upper);
        let damping6 = damping_from_cl(mu6, &c6, &d_sq6);
        let idx = non_finite_damping_indices(&damping6)[0];
        let gap = active_bound_gap(&x6, &setup.lower, &setup.upper, idx, grad6[idx]);
        let x_syn = x_near_active_bound(&x6, &setup.lower, &setup.upper, idx, gap, grad6[idx]);
        let snap = damped_gram_snapshot_for_x(&setup, &x_syn, mu6);
        assert!(
            !snap.dense_cholesky_ok,
            "synthetic slack s_i should fail dense Cholesky like iter six",
        );
    }

    /// Strict-interior projection pulls `x` off the face and restores finite damping + Cholesky.
    #[test]
    fn toy_strict_interior_projection_fixes_near_box_damping() {
        let setup = toy_joint_fixture();
        let (_, x6, mu6) = trf_replay_state_at_iter(6);
        let problem = setup.problem();
        let grad6 = problem
            .jacobian(&x6)
            .unwrap()
            .mat_transpose_vec(&problem.residual_at(&x6));
        let (d_sq6, c6) = cl_scaling(&x6, &grad6, &setup.lower, &setup.upper);
        let damping6 = damping_from_cl(mu6, &c6, &d_sq6);
        let idx = non_finite_damping_indices(&damping6)[0];
        let gap = active_bound_gap(&x6, &setup.lower, &setup.upper, idx, grad6[idx]);
        let mut x_syn = x_near_active_bound(&x6, &setup.lower, &setup.upper, idx, gap, grad6[idx]);
        let rstep = TrfReplay::default().rstep;
        project_strictly_inside_col(&mut x_syn, &setup.lower, &setup.upper, rstep);
        let snap = damped_gram_snapshot_for_x(&setup, &x_syn, mu6);
        assert!(
            snap.min_damping.is_finite() && snap.min_damping > 0.0,
            "after rstep={rstep} projection, damping should be finite",
        );
        assert!(
            relative_gap_to_active_bound(
                x_syn[idx],
                setup.lower[idx],
                setup.upper[idx],
                grad6[idx],
            ) >= rstep * 0.5,
            "projected x should sit at least O(rstep) inside active bound",
        );
        assert!(
            snap.dense_cholesky_ok,
            "interior projected iterate should yield SPD damped Gram",
        );
    }

    /// At iter six: `(upper_i − x_i)` so small that `c_i + μ d²_i` is non-finite on a
    /// capped coordinate; other coordinates still keep `max_i |g_i| · dist_i ≥ tol_grad`.
    #[test]
    fn toy_damped_gram_cl_scaling_diverges_at_iter_six() {
        let s1 = damped_gram_snapshot(1);
        assert!(
            s1.min_damping.is_finite() && s1.min_damping > 0.0,
            "iter-1 min damping should be finite",
        );
        let (setup, x, mu) = trf_replay_state_at_iter(6);
        let problem = setup.problem();
        let grad = problem
            .jacobian(&x)
            .unwrap()
            .mat_transpose_vec(&problem.residual_at(&x));
        let (d_sq, c_diag) = cl_scaling(&x, &grad, &setup.lower, &setup.upper);
        let damping = damping_from_cl(mu, &c_diag, &d_sq);
        let bad: Vec<usize> = damping
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.is_finite())
            .map(|(i, _)| i)
            .collect();
        assert!(
            !bad.is_empty(),
            "expected non-finite damping at iter 6 (μ={mu}); indices with finite d only",
        );
    }

    /// At iteration six, pinpoint whether Arrowhead or sparse fails first under carried `μ`.
    #[test]
    fn toy_trf_replay_iter_six_newton_failure_diff() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let (_, snaps, fail) =
            TrfReplay::default().run(&problem, &setup.x, &setup.lower, &setup.upper, 6);
        assert_eq!(fail, Some(6), "expected failure on iter six");
        let snap = snaps.last().expect("iter six snapshot");
        assert!(
            snap.newton.h_sparse.is_some(),
            "sparse oracle should still solve at iter six (mu_in={})",
            snap.mu_in,
        );
        assert!(
            snap.newton.h_arrow.is_none(),
            "Arrowhead should fail where replay stops (mu_in={})",
            snap.mu_in,
        );
    }

    /// First inner attempt at iter six: damped Gram is singular; sparse succeeds, Arrowhead fails.
    #[test]
    fn toy_trf_replay_iter_six_first_attempt_arrow_fails_sparse_ok() {
        let (setup, x, mu_in) = trf_replay_state_at_iter(6);
        let problem = setup.problem();
        let j = problem.jacobian(&x).unwrap();
        let r = problem.residual_at(&x);
        let grad = j.mat_transpose_vec(&r);
        let (d_sq, c_diag) = cl_scaling(&x, &grad, &setup.lower, &setup.upper);
        let damping = damping_from_cl(mu_in, &c_diag, &d_sq);
        let neg_g = Col::from_fn(x.nrows(), |i| -grad[i]);

        let mut dense = dense_jtj(&j);
        for i in 0..x.nrows() {
            dense[(i, i)] += damping[i];
        }
        let dense_ok = Llt::new(dense.as_ref(), faer::Side::Lower).is_ok();
        let sparse_j = sparse_jacobian_from_dense(&j);
        let mut sparse_gram = sparse_j.gram();
        sparse_gram.add_diagonal_vector_in_place(&damping);
        let sparse_ok = sparse_gram.solve_spd(&neg_g).is_ok();
        assert!(
            sparse_ok,
            "sparse oracle should solve at iter-six μ_in={mu_in}",
        );
        // Damped Gram is near-SPD: sparse Cholesky succeeds where dense Cholesky
        // and Arrowhead factorization report failure (iter-six regression).
        let _ = dense_ok;

        let mut gram = j.gram();
        gram.add_diagonal_vector_in_place(&damping);
        match gram.solve_spd(&neg_g) {
            Ok(_) => panic!("Arrowhead unexpectedly succeeded at iter-six μ_in={mu_in}"),
            Err(e) => {
                assert!(
                    matches!(e, basin::LinearSolveError::NotPositiveDefinite),
                    "unexpected Arrowhead error at μ_in={mu_in}: {e:?} (dense_ok={dense_ok})",
                );
            }
        }
    }

    /// Manual replay stays aligned with Basin `Executor` through five steps.
    #[test]
    fn toy_trf_replay_x_matches_executor_through_five() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let (x_replay, snaps, fail) =
            TrfReplay::default().run(&problem, &setup.x, &setup.lower, &setup.upper, 5);
        assert!(fail.is_none(), "replay failed at iter {fail:?}");
        assert_eq!(snaps.len(), 5);

        let (x_exec, reason) = trf_param_after_steps(&problem, &setup.x, 5);
        assert!(!matches!(reason, TerminationReason::SolverFailed));

        for (i, (a, b)) in x_replay.iter().zip(x_exec.iter()).enumerate() {
            assert!(
                (a - b).abs() < 1e-6,
                "x mismatch at {i}: replay={a} executor={b}",
            );
        }
        let r_exec = residual_norm(&problem, &x_exec);
        let r_replay = residual_norm(&problem, &x_replay);
        assert!(
            (r_exec - r_replay).abs() < 1e-6,
            "replay residual {r_replay} vs executor {r_exec}",
        );
    }

    fn assert_jacobian_matches_fd(problem: &PriceResidualJoint<'_>, x0: &Col<f64>) {
        let n = x0.nrows();
        let jac = problem.jacobian(x0).unwrap();
        let r0 = problem.residual_at(x0);

        for j in 0..n {
            let mut x1 = x0.clone();
            let mut h = FD_STEP.max(FD_STEP * x0[j].abs());
            if x0[j] + h > problem.upper[j] {
                h = -h;
            }
            x1[j] += h;
            let r1 = problem.residual_at(&x1);

            for i in 0..n {
                let expected = (r1[i] - r0[i]) / h;
                if expected.abs() <= 1e-12 {
                    continue;
                }
                let got = jac.get(i, j);
                assert!(
                    (got - expected).abs() < 1e-5,
                    "Jacobian mismatch at ({i}, {j}): got {got} expected {expected}",
                );
            }
        }
    }

    /// Independent FD columns match the batched + analytical [`PriceResidualJoint::jacobian`].
    #[test]
    fn toy_jacobian_matches_finite_difference_at_equilibrate_start() {
        let setup = toy_joint_fixture();
        assert_jacobian_matches_fd(&setup.problem(), &setup.x);
    }

    /// Same FD check at an interior point away from the equilibrate start.
    #[test]
    fn toy_jacobian_matches_finite_difference_at_interior_point() {
        let setup = toy_joint_fixture();
        let x0 = Col::from_fn(setup.n(), |_| 1.5);
        assert_jacobian_matches_fd(&setup.problem(), &x0);
    }

    /// [`MaxDiagonal`] on `JᵀJ + diag(c)` at the TRF start matches dense `JᵀJ`.
    #[test]
    fn toy_gram_max_diagonal_matches_dense() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let j = problem.jacobian(&setup.x).unwrap();
        let grad = j.mat_transpose_vec(&problem.residual_at(&setup.x));
        let n = setup.n();

        let mut c_diag = Col::zeros(n);
        for i in 0..n {
            c_diag[i] = cl_scaling_pair(setup.x[i], grad[i], setup.lower[i], setup.upper[i]).1;
        }

        let mut gram = j.gram();
        gram.add_diagonal_vector_in_place(&c_diag);
        let structured = gram.max_diagonal();

        let mut dense = dense_jtj(&j);
        for i in 0..n {
            dense[(i, i)] += c_diag[i];
        }
        let dense_max = (0..n)
            .map(|i| dense[(i, i)])
            .fold(f64::NEG_INFINITY, f64::max);

        assert!(
            (structured - dense_max).abs() < 1e-8,
            "max_diagonal: structured={structured} dense={dense_max}",
        );
    }

    #[test]
    fn toy_gram_solve_spd_matches_dense_at_trf_start() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let j = problem.jacobian(&setup.x).unwrap();
        let r = problem.residual_at(&setup.x);
        let grad = j.mat_transpose_vec(&r);
        let neg_g = Col::from_fn(setup.n(), |i| -grad[i]);
        let damping = trf_damping(&j, &setup.x, &grad, &setup.lower, &setup.upper, 1e-3);

        let mut gram_damped = j.gram();
        gram_damped.add_diagonal_vector_in_place(&damping);
        let x_struct = gram_damped
            .solve_spd(&neg_g)
            .expect("Arrowhead solve_spd at TRF start");

        let x_dense = dense_gram_solve(&dense_jtj(&j), &neg_g, &damping);

        for i in 0..setup.n() {
            assert!(
                (x_struct[i] - x_dense[i]).abs() < 1e-4,
                "TRF Newton step mismatch at {i}: structured={} dense={}",
                x_struct[i],
                x_dense[i],
            );
        }
    }

    #[test]
    fn toy_gram_solve_spd_matches_sparse_gram_at_trf_start() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let j = problem.jacobian(&setup.x).unwrap();
        let r = problem.residual_at(&setup.x);
        let grad = j.mat_transpose_vec(&r);
        let neg_g = Col::from_fn(setup.n(), |i| -grad[i]);
        let damping = trf_damping(&j, &setup.x, &grad, &setup.lower, &setup.upper, 1e-3);

        let mut arrow = j.gram();
        arrow.add_diagonal_vector_in_place(&damping);
        let h_arrow = arrow.solve_spd(&neg_g).expect("arrowhead step");
        let sparse_j = sparse_jacobian_from_dense(&j);
        let mut sparse_gram = sparse_j.gram();
        sparse_gram.add_diagonal_vector_in_place(&damping);
        let h_sparse = sparse_gram.solve_spd(&neg_g).expect("sparse Gram step");

        for i in 0..setup.n() {
            assert!(
                (h_arrow[i] - h_sparse[i]).abs() < 1e-4,
                "arrowhead vs sparse step at {i}: {} vs {}",
                h_arrow[i],
                h_sparse[i],
            );
        }
    }

    /// Gram oracles (max diagonal, damped Newton step) hold at every early TRF iterate.
    #[test]
    fn toy_gram_oracles_hold_through_first_five_trf_steps() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let mut x = setup.x.clone();

        for step in 1..=5_u64 {
            assert_gram_solve_matches_oracles(
                &problem,
                &x,
                &setup.lower,
                &setup.upper,
                &format!("before TRF step {step}"),
            );
            let (x_next, reason) = trf_param_after_steps(&problem, &setup.x, step);
            assert!(
                !matches!(reason, TerminationReason::SolverFailed),
                "SolverFailed during first five TRF steps at step {step}",
            );
            x = x_next;
            assert_jacobian_matches_fd(&problem, &x);
        }
    }

    /// TRF must actually move `x` and lower ‖R‖ in the first few iterations.
    #[test]
    fn toy_joint_trf_makes_progress_in_first_five_steps() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();
        let r0 = residual_norm(&problem, &setup.x);
        let (x5, reason) = trf_param_after_steps(&problem, &setup.x, 5);

        assert!(
            !matches!(reason, TerminationReason::SolverFailed),
            "unexpected SolverFailed within five steps",
        );
        assert!(
            x5.iter()
                .zip(setup.x.iter())
                .any(|(a, b)| (a - b).abs() > 1e-9),
            "TRF param unchanged after five steps",
        );
        let r5 = residual_norm(&problem, &x5);
        assert!(
            r5 < r0,
            "residual should decrease in five steps: {r0} -> {r5}",
        );
    }

    /// With BCL reflection, bound-limited coordinates no longer drive Coleman–Li
    /// `d² → ∞`. Default θ-step-back used to `SolverFailed` by step 6 on this fixture;
    /// reflection must keep iterating (or stop at scaled KKT) without that crash.
    #[test]
    fn trf_with_reflection_survives_near_box() {
        let setup = toy_joint_fixture();
        let problem = setup.problem();

        let (_, reason5) = trf_param_after_steps(&problem, &setup.x, 5);
        assert!(
            !matches!(reason5, TerminationReason::SolverFailed),
            "five TRF steps should succeed; got {reason5:?}",
        );
        let (_, reason6) = trf_param_after_steps(&problem, &setup.x, 6);
        assert!(
            !matches!(reason6, TerminationReason::SolverFailed),
            "step six must not SolverFailed with reflection; got {reason6:?}",
        );

        let (x, reason) = trf_param_after_steps(&problem, &setup.x, 200);
        assert!(
            !matches!(reason, TerminationReason::SolverFailed),
            "reflection TRF must not exhaust μ / Cholesky; got {reason:?}",
        );
        assert!(
            x.iter().all(|v| v.is_finite()),
            "param must stay finite; reason={reason:?} x={x:?}",
        );
        let r = residual_norm(&problem, &x);
        assert!(r.is_finite(), "residual must stay finite; got {r}");
    }

    /// Full toy joint solve via [`equilibrate_joint`].
    ///
    /// Asserts Basin does not `SolverFailed` (reflection keeps face coords finite).
    /// Does **not** require [`SolveStatus::Converged`]: when unclipped τ lies outside
    /// the price box, a face-active KKT with nonzero residual is the correct capped
    /// outcome (disequilibrium; see `docs/prices-equilibrium.md`), and Vic3 reports
    /// [`SolveStatus::MaxIters`].
    #[test]
    fn toy_joint_equilibrate_finishes() {
        let setup = toy_joint_fixture();
        let (outcome, _) = equilibrate_joint(
            setup.cache.as_ref(),
            &setup.defs,
            SolveOpts {
                strategy: SolveStrategy::Joint,
                ..Default::default()
            },
        );
        assert_ne!(
            outcome.status,
            SolveStatus::Failed,
            "joint TRF must not SolverFailed; outcome={outcome:?}",
        );
        assert!(
            outcome.residual.is_finite(),
            "residual must be finite; got {}",
            outcome.residual,
        );
        assert!(
            outcome.relative.iter().all(|p| p.is_finite()),
            "relative prices must stay finite",
        );
    }

    proptest! {
        /// Damped Newton step on the toy TRF start matches dense/sparse for random `μ` scale.
        #[test]
        fn proptest_toy_damped_solve_matches_oracles(log_scale in -4.0f64..0.0) {
            let scale = 10f64.powf(log_scale);
            let setup = toy_joint_fixture();
            let problem = setup.problem();
            let n = setup.n();
            let damping = Col::from_fn(n, |i| scale * (1.0 + 0.01 * i as f64));
            assert_damped_solve_matches_oracles(
                &problem,
                &setup.x,
                &damping,
                "proptest toy TRF start",
            );
        }
    }
}
