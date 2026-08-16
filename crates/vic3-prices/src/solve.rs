//! Bound-constrained NLS (`basin::Trf`) plus successive-substitution warm start.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use basin::{
    BoxConstraints, CostFunction, DenseMatrix, Executor, Jacobian, Residual, TerminationReason, Trf,
};
use vic3_defs::GameDefs;

use crate::consumption::consumption;
use crate::formula::price;
use crate::result::{
    BuildingEconomics, CountryInfo, GoodFlow, GoodPrice, MarketInputs, PricesResult, SolveOpts,
    SolveStatus, StateGood, StateInfo,
};
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
        return finished(world, defs, Vec::new(), 0.0, SolveStatus::Converged);
    }

    let bases: Vec<f64> = goods
        .iter()
        .map(|id| defs.base_price(id).unwrap_or(0.0))
        .collect();
    if bases.iter().any(|b| *b <= 0.0) {
        return finished(world, defs, Vec::new(), f64::INFINITY, SolveStatus::Failed);
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

    finished(world, defs, rows, residual, status)
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

fn finished(
    world: &World,
    defs: &GameDefs,
    goods: Vec<GoodPrice>,
    residual: f64,
    status: SolveStatus,
) -> PricesResult {
    let (states, state_goods, buildings) = detail_rows(world, defs, &goods);
    let countries = country_rows(world, defs);
    let inputs = MarketInputs {
        pops: world.pops.len(),
        skipped_pops: world.skipped_pops,
        buildings: world.buildings.len(),
        skipped_buildings: world.skipped_buildings,
        buildings_without_method: world
            .buildings
            .iter()
            .filter(|building| !building.has_known_method(defs))
            .count(),
        goods_with_orders: goods
            .iter()
            .filter(|good| good.buy > crate::ORDER_EPS || good.sell > crate::ORDER_EPS)
            .count(),
    };
    PricesResult {
        scope: "whole_save_synthetic".to_string(),
        goods,
        countries,
        states,
        state_goods,
        buildings,
        inputs,
        residual,
        status,
        limitations: LIMITATIONS.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn detail_rows(
    world: &World,
    defs: &GameDefs,
    goods: &[GoodPrice],
) -> (Vec<StateInfo>, Vec<StateGood>, Vec<BuildingEconomics>) {
    let prices = goods
        .iter()
        .map(|good| (good.id.clone(), good.price))
        .collect::<BTreeMap<_, _>>();
    let rows = goods
        .iter()
        .map(|good| (good.id.as_str(), good))
        .collect::<BTreeMap<_, _>>();
    let mut state_buy = BTreeMap::<(u32, String), f64>::new();
    let mut state_sell = BTreeMap::<(u32, String), f64>::new();

    for state in &world.states {
        let pops = world
            .pops
            .iter()
            .filter(|pop| pop.state == Some(state.id))
            .cloned()
            .collect::<Vec<_>>();
        for (good, quantity) in consumption(&pops, &prices, defs, &world.frozen_sell) {
            *state_buy.entry((state.id, good)).or_default() += quantity;
        }
    }

    let mut buildings = Vec::new();
    for building in &world.buildings {
        let scale = building.level * building.staffing;
        let mut input_qty = BTreeMap::<String, f64>::new();
        let mut output_qty = BTreeMap::<String, f64>::new();
        let methods = building.methods(defs);
        if methods.is_empty() {
            input_qty.extend(building.saved_inputs.clone());
            output_qty.extend(building.saved_outputs.clone());
        } else {
            for method in methods {
                for (good_id, per_level) in &method.inputs {
                    *input_qty.entry(good_id.clone()).or_default() += per_level * scale;
                }
                for (good_id, per_level) in &method.outputs {
                    *output_qty.entry(good_id.clone()).or_default() += per_level * scale;
                }
            }
        }
        let inputs = priced_flows(input_qty, &prices, building.state, &mut state_buy);
        let outputs = priced_flows(output_qty, &prices, building.state, &mut state_sell);
        let cost = inputs.iter().map(|flow| flow.value).sum::<f64>();
        let revenue = outputs.iter().map(|flow| flow.value).sum::<f64>();
        let short_inputs = inputs
            .iter()
            .filter(|flow| {
                rows.get(flow.good_id.as_str()).is_none_or(|row| {
                    row.sell <= crate::ORDER_EPS
                        || row.price
                            >= row.base * (1.0 + defs.price_range.max(0.0)) - crate::ORDER_EPS
                })
            })
            .map(|flow| flow.good_id.clone())
            .collect();
        buildings.push(BuildingEconomics {
            id: building.id,
            state_id: building.state,
            type_id: building.building.clone(),
            level: building.level,
            staffing: building.staffing,
            production_method_ids: building.production_methods.clone(),
            inputs,
            outputs,
            revenue,
            cost,
            profit: revenue - cost,
            short_inputs,
        });
    }

    let mut keys = world
        .states
        .iter()
        .flat_map(|state| goods.iter().map(move |good| (state.id, good.id.clone())))
        .collect::<BTreeSet<_>>();
    keys.extend(state_buy.keys().chain(state_sell.keys()).cloned());
    let state_goods = keys
        .into_iter()
        .filter_map(|(state_id, good_id)| {
            let row = rows.get(good_id.as_str())?;
            Some(StateGood {
                state_id,
                good_id: good_id.clone(),
                buy: state_buy
                    .get(&(state_id, good_id.clone()))
                    .copied()
                    .unwrap_or(0.0),
                sell: state_sell.get(&(state_id, good_id)).copied().unwrap_or(0.0),
                price: row.price,
                base: row.base,
            })
        })
        .collect();
    let states = world
        .states
        .iter()
        .map(|state| StateInfo {
            id: state.id,
            region_id: state.region.clone(),
            country_id: state.country,
            market_id: state.market,
        })
        .collect();
    (states, state_goods, buildings)
}

fn country_rows(world: &World, defs: &GameDefs) -> Vec<CountryInfo> {
    world
        .countries
        .iter()
        .map(|country| {
            let flag_coa = vic3_defs::select_flag_coa(
                &defs.flag_defs,
                &defs.flags,
                &country.tag,
                &country.laws,
            );
            let flag_data_url = flag_coa.as_ref().and_then(|coa| {
                defs.flags
                    .get(coa)
                    .map(|png| format!("data:image/png;base64,{}", base64_encode(png)))
            });
            CountryInfo {
                id: country.id,
                tag: country.tag.clone(),
                name: defs.labels.get(&country.tag).cloned(),
                flag_coa,
                flag_data_url,
            }
        })
        .collect()
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (a << 16) | (b << 8) | c;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Value one side of a building's goods flows, also crediting the quantities to
/// the building's state.
fn priced_flows(
    quantities: BTreeMap<String, f64>,
    prices: &BTreeMap<String, f64>,
    state: Option<u32>,
    state_side: &mut BTreeMap<(u32, String), f64>,
) -> Vec<GoodFlow> {
    quantities
        .into_iter()
        .map(|(good_id, quantity)| {
            if let Some(state_id) = state {
                *state_side.entry((state_id, good_id.clone())).or_default() += quantity;
            }
            GoodFlow {
                value: prices.get(&good_id).copied().unwrap_or(0.0) * quantity,
                good_id,
                quantity,
            }
        })
        .collect()
}

fn market_goods(world: &World, defs: &GameDefs) -> Vec<String> {
    let mut ids: BTreeSet<String> = defs.goods.keys().cloned().collect();
    ids.extend(world.frozen_buy.keys().cloned());
    ids.extend(world.frozen_sell.keys().cloned());
    for b in &world.buildings {
        for pm in b.methods(defs) {
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
                    name: self.defs.labels.get(id).cloned(),
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
