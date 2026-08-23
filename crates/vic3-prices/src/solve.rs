//! Bound-constrained NLS ([`basin::Trf`]) plus successive-substitution warm start.
//!
//! # Algorithm (same [`equilibrate`] / [`solve`] API)
//!
//! 1. Build access-scaled frozen non-pop orders and state shops (pops at local
//!    prices; buildings + post-1.9 trade frozen).
//! 2. If [`SolveOpts::warm_rel`] length matches, clamp and start Basin from it
//!    (skip step 3). Else run successive substitution
//!    `r ← (1−α)r + α P(c(r))` for a few iterations (`α = 0.5`).
//! 3. Run Basin trust-region-reflective on
//!    `‖r − r_formula(orders(r))‖²` with box bounds from `PRICE_RANGE`.
//! 4. Polish with successive substitution (also the fallback after
//!    `SolverFailed`). TRF stays strictly inside the box; SS may sit on a bound.
//!
//! [`equilibrate`] returns a compact [`SolveOutcome`] (goods, residual, relative,
//! building revenues). [`solve`] packages that into a full [`PricesResult`] via
//! [`crate::report`] for CLI / UI / SQL. Planning should call [`equilibrate`].
//!
//! Downstream: [`PricesResult`] feeds `vic3-api` JSON; see the crate root docs.

use std::collections::BTreeMap;
use std::convert::Infallible;

use basin::{
    BoxConstraints, CostFunction, DenseMatrix, Executor, Jacobian, Residual, TerminationReason, Trf,
};
use vic3_defs::{GameDefs, GoodIdx, GoodsVec};

use crate::consumption::{
    add_pop_from_units, add_wage_bins, wage_bins_from_pops, NeedShares, UnitBaskets, WealthBin,
};
use crate::formula::{effective_mapi, local_price, market_access, price};
use crate::report::{building_revenues, report_from_solve};
use crate::result::{GoodPrice, PricesResult, SolveOpts, SolveOutcome, SolveStatus};
use crate::world::{World, WorldPop};

const WARM_START_ALPHA: f64 = 0.5;
const FD_STEP: f64 = 1e-7;
const LOCAL_ITERS: u32 = 16;
const LOCAL_EPS: f64 = 1e-10;

/// Find relative prices `r` minimizing `‖r − r_formula(orders(r))‖²`
/// with box bounds `r ∈ [1 − PRICE_RANGE, 1 + PRICE_RANGE]`.
///
/// Returns a compact [`SolveOutcome`] without building the full UI/SQL tables.
/// Use [`solve`] when callers need [`PricesResult`].
pub fn equilibrate(world: &World, defs: &GameDefs, opts: SolveOpts) -> SolveOutcome {
    equilibrate_inner(world, defs, opts).0
}

/// Full public solve: [`equilibrate`] then [`crate::report`].
pub fn solve(world: &World, defs: &GameDefs, opts: SolveOpts) -> PricesResult {
    let (outcome, snapshot) = equilibrate_inner(world, defs, opts);
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
    let next = world.with_extra_levels(&delta.building, delta.extra_levels);
    solve(&next, defs, opts)
}

fn equilibrate_inner(
    world: &World,
    defs: &GameDefs,
    opts: SolveOpts,
) -> (SolveOutcome, Option<ShopSnapshot>) {
    let base_prices: GoodsVec = defs
        .goods_order
        .iter()
        .map(|id| defs.base_price(id).unwrap_or(0.0))
        .collect();
    let goods = market_goods(&base_prices);
    if goods.is_empty() {
        return (
            SolveOutcome {
                goods: Vec::new(),
                residual: 0.0,
                status: SolveStatus::Converged,
                relative: Vec::new(),
                building_revenues: Vec::new(),
            },
            None,
        );
    }

    let bases: Vec<f64> = goods.iter().map(|&idx| base_prices[idx]).collect();
    if bases.iter().any(|b| *b <= 0.0) {
        return (
            SolveOutcome {
                goods: Vec::new(),
                residual: f64::INFINITY,
                status: SolveStatus::Failed,
                relative: Vec::new(),
                building_revenues: Vec::new(),
            },
            None,
        );
    }

    let price_range = defs.price_range.max(0.0);
    let n = goods.len();
    let access_by_state = world
        .states
        .iter()
        .map(|state| {
            (
                state.id,
                market_access(state.infrastructure, state.infrastructure_usage),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let (frozen_buy, frozen_sell) = access_scaled_non_pop_orders(world, defs, &access_by_state);
    let n_goods = defs.goods_order.len();
    let shares = NeedShares::from_sell(defs, &frozen_sell);
    let units = UnitBaskets::from_shares(defs, &base_prices, &shares);
    let shops = state_shops(world, defs, &access_by_state, n_goods, &base_prices, &units);
    let (stateless_wage_bins, frozen_pop_buy) =
        split_pop_buy(world, n_goods, &base_prices, &units, &shops);
    let problem = PriceResidual {
        defs,
        goods: &goods,
        bases: &bases,
        base_prices,
        price_range,
        lower: vec![1.0 - price_range; n],
        upper: vec![1.0 + price_range; n],
        frozen_buy,
        frozen_sell,
        units,
        shops,
        stateless_wage_bins,
        frozen_pop_buy,
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

    let (rows, residual, snapshot) = problem.evaluate(&rel);
    let status = if residual < opts.residual_eps {
        SolveStatus::Converged
    } else if failed && !used_max_iters {
        SolveStatus::Failed
    } else {
        SolveStatus::MaxIters
    };

    let building_revenues = building_revenues(world, defs, &rows, Some(&snapshot));
    (
        SolveOutcome {
            goods: rows,
            residual,
            status,
            relative: rel,
            building_revenues,
        },
        Some(snapshot),
    )
}

fn market_goods(base_prices: &GoodsVec) -> Vec<GoodIdx> {
    base_prices
        .iter_indexed()
        .filter(|(_, base)| *base > 0.0)
        .map(|(idx, _)| idx)
        .collect()
}

/// Non-pop orders reaching the single market after state access scaling.
///
/// Post-1.9 trade and buildings are attributed to their states. Buildings
/// without a state remain global at 100%.
fn access_scaled_non_pop_orders(
    world: &World,
    defs: &GameDefs,
    access_by_state: &BTreeMap<u32, f64>,
) -> (GoodsVec, GoodsVec) {
    let n = defs.goods_order.len();
    let mut buy = world.frozen_buy.aligned(n);
    let mut sell = world.frozen_sell.aligned(n);
    for trade in &world.state_trade {
        let access = access_by_state.get(&trade.state).copied().unwrap_or(1.0);
        trade.add_orders(&mut buy, &mut sell, access);
    }
    for building in &world.buildings {
        let access = building
            .state
            .and_then(|state| access_by_state.get(&state).copied())
            .unwrap_or(1.0);
        let (inputs, outputs) = building.goods_io(defs);
        for (good, quantity) in inputs.iter_indexed() {
            buy.add(good, quantity * access);
        }
        for (good, quantity) in outputs.iter_indexed() {
            sell.add(good, quantity * access);
        }
    }
    (buy, sell)
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

#[derive(Clone)]
struct StateShop {
    id: u32,
    access: f64,
    mapi: f64,
    frozen_buy: GoodsVec,
    frozen_sell: GoodsVec,
    frozen_pop_buy: GoodsVec,
    wage_bins: Vec<WealthBin>,
}

#[derive(Clone, Default)]
pub(crate) struct ShopSnapshot {
    pub(crate) world_pop_buy: GoodsVec,
    pub(crate) local_by_state: BTreeMap<u32, GoodsVec>,
    pub(crate) pop_buy_by_state: BTreeMap<u32, GoodsVec>,
}

fn state_shops(
    world: &World,
    defs: &GameDefs,
    access_by_state: &BTreeMap<u32, f64>,
    n: usize,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
) -> Vec<StateShop> {
    let pops: Vec<WorldPop> = world.iter_pops().collect();
    let mut ids: BTreeMap<u32, f64> = access_by_state.clone();
    for state in &world.states {
        ids.entry(state.id)
            .or_insert_with(|| market_access(state.infrastructure, state.infrastructure_usage));
    }
    for pop in &pops {
        if let Some(state) = pop.state {
            ids.entry(state).or_insert(1.0);
        }
    }
    for building in &world.buildings {
        if let Some(state) = building.state {
            ids.entry(state).or_insert(1.0);
        }
    }
    for trade in &world.state_trade {
        ids.entry(trade.state).or_insert(1.0);
    }

    ids.into_iter()
        .map(|(id, access)| {
            let mut frozen_buy = GoodsVec::zeros(n);
            let mut frozen_sell = GoodsVec::zeros(n);
            for trade in &world.state_trade {
                if trade.state == id {
                    trade.add_orders(&mut frozen_buy, &mut frozen_sell, 1.0);
                }
            }
            for building in &world.buildings {
                if building.state == Some(id) {
                    let (inputs, outputs) = building.goods_io(defs);
                    for (good, quantity) in inputs.iter_indexed() {
                        frozen_buy.add(good, quantity);
                    }
                    for (good, quantity) in outputs.iter_indexed() {
                        frozen_sell.add(good, quantity);
                    }
                }
            }
            let mut frozen_pop_buy = GoodsVec::zeros(n);
            let mut wage_pops = Vec::new();
            for pop in &pops {
                if pop.state != Some(id) {
                    continue;
                }
                if pop.wages > 0.0 {
                    wage_pops.push(pop);
                } else {
                    add_pop_from_units(
                        &mut frozen_pop_buy,
                        pop,
                        base_prices,
                        base_prices,
                        units,
                        1.0,
                    );
                }
            }
            StateShop {
                id,
                access,
                mapi: effective_mapi(access),
                frozen_buy,
                frozen_sell,
                frozen_pop_buy,
                wage_bins: wage_bins_from_pops(wage_pops),
            }
        })
        .collect()
}

/// Split pops with no state into wage-sensitive vs frozen-wealth; precompute
/// access-scaled frozen pop buy (stateless + every state's frozen pops).
fn split_pop_buy(
    world: &World,
    n: usize,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    shops: &[StateShop],
) -> (Vec<WealthBin>, GoodsVec) {
    let mut frozen_pop_buy = GoodsVec::zeros(n);
    let mut stateless = Vec::new();
    for pop in world.iter_pops() {
        if pop.state.is_some() {
            continue;
        }
        if pop.wages > 0.0 {
            stateless.push(pop);
        } else {
            add_pop_from_units(
                &mut frozen_pop_buy,
                &pop,
                base_prices,
                base_prices,
                units,
                1.0,
            );
        }
    }
    for shop in shops {
        for (good, quantity) in shop.frozen_pop_buy.iter_indexed() {
            frozen_pop_buy.add(good, quantity * shop.access);
        }
    }
    (wage_bins_from_pops(&stateless), frozen_pop_buy)
}

#[derive(Clone)]
struct PriceResidual<'a> {
    defs: &'a GameDefs,
    goods: &'a [GoodIdx],
    bases: &'a [f64],
    base_prices: GoodsVec,
    price_range: f64,
    lower: Vec<f64>,
    upper: Vec<f64>,
    frozen_buy: GoodsVec,
    frozen_sell: GoodsVec,
    units: UnitBaskets,
    shops: Vec<StateShop>,
    stateless_wage_bins: Vec<WealthBin>,
    frozen_pop_buy: GoodsVec,
}

impl PriceResidual<'_> {
    fn prices_from_rel(&self, rel: &[f64]) -> GoodsVec {
        let mut prices = self.base_prices.clone();
        for (&good, (&base, &r)) in self.goods.iter().zip(self.bases.iter().zip(rel)) {
            prices[good] = base * r;
        }
        prices
    }

    fn settle_state(&self, shop: &StateShop, market: &GoodsVec, scratch: &mut SettleScratch) {
        let n = self.base_prices.len();
        scratch.local.copy_from(market);
        scratch.pop_buy.copy_from(&shop.frozen_pop_buy);
        for _ in 0..LOCAL_ITERS {
            scratch.pop_buy.copy_from(&shop.frozen_pop_buy);
            add_wage_bins(
                &mut scratch.pop_buy,
                &shop.wage_bins,
                &scratch.local,
                &self.base_prices,
                &self.units,
                1.0,
            );
            let mut delta = 0.0_f64;
            for i in 0..n {
                let good = GoodIdx::from_usize(i);
                let buy = shop.frozen_buy[good] + scratch.pop_buy[good];
                let sell = shop.frozen_sell[good];
                let state_price = price(self.base_prices[good], buy, sell, self.price_range);
                let price = local_price(shop.mapi, market[good], state_price);
                delta = delta.max((price - scratch.local[good]).abs());
                scratch.next[good] = price;
            }
            scratch.local.copy_from(&scratch.next);
            if delta < LOCAL_EPS {
                break;
            }
        }
    }

    fn world_pop_buy_at(&self, market: &GoodsVec, scratch: &mut SettleScratch) -> GoodsVec {
        let mut buy = self.frozen_pop_buy.clone();
        add_wage_bins(
            &mut buy,
            &self.stateless_wage_bins,
            market,
            &self.base_prices,
            &self.units,
            1.0,
        );
        for shop in &self.shops {
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

    fn snapshot_at(&self, market: &GoodsVec, scratch: &mut SettleScratch) -> ShopSnapshot {
        let mut world_pop_buy = self.frozen_pop_buy.clone();
        add_wage_bins(
            &mut world_pop_buy,
            &self.stateless_wage_bins,
            market,
            &self.base_prices,
            &self.units,
            1.0,
        );
        let mut local_by_state = BTreeMap::new();
        let mut pop_buy_by_state = BTreeMap::new();
        for shop in &self.shops {
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

    fn pop_buy_at(&self, prices: &GoodsVec, scratch: &mut SettleScratch) -> GoodsVec {
        self.world_pop_buy_at(prices, scratch)
    }

    fn formula_rel(&self, rel: &[f64], scratch: &mut SettleScratch) -> Vec<f64> {
        let prices = self.prices_from_rel(rel);
        let pop_buy = self.pop_buy_at(&prices, scratch);
        self.goods
            .iter()
            .zip(self.bases)
            .map(|(&id, base)| {
                let buy = self.frozen_buy[id] + pop_buy[id];
                let sell = self.frozen_sell[id];
                price(*base, buy, sell, self.price_range) / *base
            })
            .collect()
    }

    fn residual_at(&self, rel: &[f64], scratch: &mut SettleScratch) -> Vec<f64> {
        self.formula_rel(rel, scratch)
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
        let mut scratch = SettleScratch::new(self.base_prices.len());
        for _ in 0..iters {
            let formula = self.formula_rel(rel, &mut scratch);
            for (r, f) in rel.iter_mut().zip(formula) {
                *r = (1.0 - alpha) * *r + alpha * f;
            }
            self.clamp_rel(rel);
        }
    }

    fn evaluate(&self, rel: &[f64]) -> (Vec<GoodPrice>, f64, ShopSnapshot) {
        let mut scratch = SettleScratch::new(self.base_prices.len());
        let prices = self.prices_from_rel(rel);
        let snapshot = self.snapshot_at(&prices, &mut scratch);
        let pop_buy = &snapshot.world_pop_buy;
        let residual = self
            .goods
            .iter()
            .zip(self.bases.iter().zip(rel.iter()))
            .map(|(&id, (base, rrel))| {
                let buy = self.frozen_buy[id] + pop_buy[id];
                let sell = self.frozen_sell[id];
                let formula = price(*base, buy, sell, self.price_range) / *base;
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
                let buy = self.frozen_buy[id] + pop_buy[id];
                let sell = self.frozen_sell[id];
                Some(GoodPrice {
                    id: good_id.to_string(),
                    name: self.defs.labels.get(good_id).cloned(),
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
        let mut scratch = SettleScratch::new(self.base_prices.len());
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
        let mut scratch = SettleScratch::new(self.base_prices.len());
        Ok(self.residual_at(param, &mut scratch))
    }
}

impl Jacobian for PriceResidual<'_> {
    type Jacobian = DenseMatrix<f64>;

    fn jacobian(&self, param: &Vec<f64>) -> Result<DenseMatrix<f64>, Infallible> {
        let mut scratch = SettleScratch::new(self.base_prices.len());
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
