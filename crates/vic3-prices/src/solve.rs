//! Bound-constrained NLS (`basin::Trf`) plus successive-substitution warm start.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use basin::{
    BoxConstraints, CostFunction, DenseMatrix, Executor, Jacobian, Residual, TerminationReason, Trf,
};
use vic3_defs::GameDefs;

use crate::consumption::consumption;
use crate::formula::price;
use crate::result::{GoodPrice, PricesResult, SolveOpts, SolveStatus};
use crate::world::{reconstruct_non_pop_orders, World};
use crate::LIMITATIONS;

const WARM_START_ALPHA: f64 = 0.5;
const FD_STEP: f64 = 1e-7;

/// Find relative prices `r` minimizing `‖r − r_formula(orders(r))‖²`
/// with box bounds `r ∈ [1 − PRICE_RANGE, 1 + PRICE_RANGE]`.
///
/// Warm-starts (and, on Basin failure, falls back to) successive substitution
/// `r ← (1−α)r + α P(c(r))` inside this same API.
///
/// # Limitations
///
/// Same list as [`LIMITATIONS`]:
///
/// 1. Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.
/// 2. Prices are clamped to ±PRICE_RANGE; the clamp is part of the model.
/// 3. Employment, wages, and trade volumes are frozen except explicit what-if deltas.
/// 4. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.
pub fn solve(world: &World, defs: &GameDefs, opts: SolveOpts) -> PricesResult {
    let goods = market_goods(world, defs);
    if goods.is_empty() {
        return finished(Vec::new(), 0.0, SolveStatus::Converged);
    }

    let bases: Vec<f64> = goods
        .iter()
        .map(|id| defs.base_price(id).unwrap_or(0.0))
        .collect();
    if bases.iter().any(|b| *b <= 0.0) {
        return finished(Vec::new(), f64::INFINITY, SolveStatus::Failed);
    }

    let price_range = defs.price_range.max(0.0);
    let n = goods.len();
    let (frozen_buy, frozen_sell) = reconstruct_non_pop_orders(world, defs);
    let problem = PriceResidual {
        world,
        defs,
        goods: &goods,
        bases: &bases,
        price_range,
        lower: vec![1.0 - price_range; n],
        upper: vec![1.0 + price_range; n],
        frozen_buy,
        frozen_sell,
    };

    let mut rel = vec![1.0; n];
    let warm_iters = opts.max_iters.clamp(1, 16);
    problem.damp_toward_formula(&mut rel, WARM_START_ALPHA, warm_iters);

    let mut used_max_iters = false;
    let mut failed = false;

    if price_range > 0.0 {
        let basin_iters = u64::from(opts.max_iters.saturating_sub(warm_iters)).max(1);
        match Executor::from_start(problem.clone(), Trf::new(), rel.clone())
            .max_iter(basin_iters)
            .run()
        {
            Ok(outcome) => {
                rel.clone_from(outcome.param());
                match outcome.reason {
                    TerminationReason::MaxIter => used_max_iters = true,
                    TerminationReason::SolverFailed => failed = true,
                    _ => {}
                }
            }
            Err(e) => match e {},
        }
    }

    // TRF stays strictly inside the box; successive substitution may sit on a
    // closed bound and is also the fallback after SolverFailed.
    let polish = if failed { opts.max_iters } else { warm_iters };
    problem.damp_toward_formula(&mut rel, WARM_START_ALPHA, polish);
    problem.clamp_rel(&mut rel);

    let (rows, residual) = problem.evaluate(&rel);
    let status = if residual < opts.residual_eps {
        SolveStatus::Converged
    } else if failed && !used_max_iters {
        SolveStatus::Failed
    } else {
        SolveStatus::MaxIters
    };

    finished(rows, residual, status)
}

/// Apply a building-level delta and re-solve. Employment (`staffing`) stays frozen.
pub fn what_if(
    world: &World,
    defs: &GameDefs,
    delta: &crate::result::WhatIfOpts,
    opts: SolveOpts,
) -> PricesResult {
    let next = world.with_extra_levels(&delta.building, delta.extra_levels);
    solve(&next, defs, opts)
}

fn finished(goods: Vec<GoodPrice>, residual: f64, status: SolveStatus) -> PricesResult {
    PricesResult {
        goods,
        residual,
        status,
        limitations: LIMITATIONS.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn market_goods(world: &World, defs: &GameDefs) -> Vec<String> {
    let mut ids: BTreeSet<String> = defs.goods.keys().cloned().collect();
    ids.extend(world.frozen_buy.keys().cloned());
    ids.extend(world.frozen_sell.keys().cloned());
    for b in &world.buildings {
        if let Some(pm) = defs.production_methods.get(&b.production_method) {
            ids.extend(pm.inputs.keys().cloned());
            ids.extend(pm.outputs.keys().cloned());
        }
    }
    ids.into_iter()
        .filter(|id| defs.base_price(id).is_some_and(|b| b > 0.0))
        .collect()
}

#[derive(Clone)]
struct PriceResidual<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    goods: &'a [String],
    bases: &'a [f64],
    price_range: f64,
    lower: Vec<f64>,
    upper: Vec<f64>,
    frozen_buy: BTreeMap<String, f64>,
    frozen_sell: BTreeMap<String, f64>,
}

impl PriceResidual<'_> {
    fn prices_from_rel(&self, rel: &[f64]) -> BTreeMap<String, f64> {
        self.goods
            .iter()
            .zip(self.bases.iter().zip(rel.iter()))
            .map(|(id, (base, r))| (id.clone(), *base * *r))
            .collect()
    }

    fn formula_rel(&self, rel: &[f64]) -> Vec<f64> {
        let prices = self.prices_from_rel(rel);
        let pop_buy = consumption(&self.world.pops, &prices, self.defs, &self.frozen_sell);
        self.goods
            .iter()
            .zip(self.bases)
            .map(|(id, base)| {
                let buy = self.frozen_buy.get(id).copied().unwrap_or(0.0)
                    + pop_buy.get(id).copied().unwrap_or(0.0);
                let sell = self.frozen_sell.get(id).copied().unwrap_or(0.0);
                price(*base, buy, sell, self.price_range) / *base
            })
            .collect()
    }

    fn residual_at(&self, rel: &[f64]) -> Vec<f64> {
        self.formula_rel(rel)
            .into_iter()
            .zip(rel)
            .map(|(formula, r)| r - formula)
            .collect()
    }

    fn clamp_rel(&self, rel: &mut [f64]) {
        for (i, r) in rel.iter_mut().enumerate() {
            *r = r.clamp(self.lower[i], self.upper[i]);
        }
    }

    fn damp_toward_formula(&self, rel: &mut [f64], alpha: f64, iters: u32) {
        for _ in 0..iters {
            let formula = self.formula_rel(rel);
            for (r, f) in rel.iter_mut().zip(formula) {
                *r = (1.0 - alpha) * *r + alpha * f;
            }
            self.clamp_rel(rel);
        }
    }

    fn evaluate(&self, rel: &[f64]) -> (Vec<GoodPrice>, f64) {
        let prices = self.prices_from_rel(rel);
        let pop_buy = consumption(&self.world.pops, &prices, self.defs, &self.frozen_sell);
        let residual = self
            .residual_at(rel)
            .iter()
            .map(|x| x * x)
            .sum::<f64>()
            .sqrt();
        let rows = self
            .goods
            .iter()
            .zip(self.bases.iter().zip(rel.iter()))
            .map(|(id, (base, rrel))| {
                let buy = self.frozen_buy.get(id).copied().unwrap_or(0.0)
                    + pop_buy.get(id).copied().unwrap_or(0.0);
                let sell = self.frozen_sell.get(id).copied().unwrap_or(0.0);
                GoodPrice {
                    id: id.clone(),
                    base: *base,
                    price: *base * *rrel,
                    buy,
                    sell,
                }
            })
            .collect();
        (rows, residual)
    }
}

impl CostFunction for PriceResidual<'_> {
    type Param = Vec<f64>;
    type Output = f64;
    type Error = Infallible;

    fn cost(&self, param: &Vec<f64>) -> Result<f64, Infallible> {
        Ok(0.5 * self.residual_at(param).iter().map(|x| x * x).sum::<f64>())
    }
}

impl Residual for PriceResidual<'_> {
    type Param = Vec<f64>;
    type Output = Vec<f64>;
    type Error = Infallible;

    fn residual(&self, param: &Vec<f64>) -> Result<Vec<f64>, Infallible> {
        Ok(self.residual_at(param))
    }
}

impl Jacobian for PriceResidual<'_> {
    type Jacobian = DenseMatrix<f64>;

    fn jacobian(&self, param: &Vec<f64>) -> Result<DenseMatrix<f64>, Infallible> {
        let r0 = self.residual_at(param);
        let n = param.len();
        let m = r0.len();
        let mut data = vec![0.0; m * n];
        for j in 0..n {
            let h = FD_STEP.max(FD_STEP * param[j].abs());
            let mut stepped = param.clone();
            let (x1, denom) = if param[j] + h <= self.upper[j] {
                stepped[j] = param[j] + h;
                (self.residual_at(&stepped), h)
            } else {
                stepped[j] = param[j] - h;
                (self.residual_at(&stepped), -h)
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
