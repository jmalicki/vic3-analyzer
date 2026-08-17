//! Bound-constrained NLS (`basin::Trf`) plus successive-substitution warm start.

use std::collections::BTreeMap;
use std::convert::Infallible;

use basin::{
    BoxConstraints, CostFunction, DenseMatrix, Executor, Jacobian, Residual, TerminationReason, Trf,
};
use vic3_defs::{GameDefs, GoodIdx, GoodsVec};

use crate::consumption::{add_pop_consumption_scaled, consumption, pop_need_baskets};
use crate::formula::{local_price, market_access, price};
use crate::result::{
    BuildingEconomics, BuildingGroupInfo, BuildingTypeInfo, CountryInfo, GoodFlow, GoodPrice,
    MarketInputs, PopNeedBasket, PricesResult, ProfessionCount, SolveOpts, SolveStatus, StateGood,
    StateInfo, StateNeed, StatePop, StateQualification,
};
use crate::world::{World, WorldPop, WorldStatePop};
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
/// 4. State building, pop, and post-1.9 trade orders are access-scaled into one whole-save market; full MAPI modifiers are unavailable.
/// 5. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.
pub fn solve(world: &World, defs: &GameDefs, opts: SolveOpts) -> PricesResult {
    let base_prices: GoodsVec = defs
        .goods_order
        .iter()
        .map(|id| defs.base_price(id).unwrap_or(0.0))
        .collect();
    let goods = market_goods(&base_prices);
    if goods.is_empty() {
        return finished(world, defs, Vec::new(), 0.0, SolveStatus::Converged);
    }

    let bases: Vec<f64> = goods.iter().map(|&idx| base_prices[idx]).collect();
    if bases.iter().any(|b| *b <= 0.0) {
        return finished(world, defs, Vec::new(), f64::INFINITY, SolveStatus::Failed);
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
    let pop_access = world
        .pops
        .iter()
        .map(|pop| {
            pop.state
                .and_then(|state| access_by_state.get(&state).copied())
                .unwrap_or(1.0)
        })
        .collect::<Vec<_>>();
    let (frozen_buy, frozen_sell) = access_scaled_non_pop_orders(world, defs, &access_by_state);
    let (wage_pop_idxs, frozen_pop_buy) =
        split_pop_buy(world, defs, &base_prices, &frozen_sell, &pop_access);
    let problem = PriceResidual {
        world,
        defs,
        goods: &goods,
        bases: &bases,
        base_prices,
        price_range,
        lower: vec![1.0 - price_range; n],
        upper: vec![1.0 + price_range; n],
        frozen_buy,
        frozen_sell,
        wage_pop_idxs,
        frozen_pop_buy,
        pop_access,
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
    let detail = detail_rows(world, defs, &goods);
    let countries = country_rows(world, defs);
    let building_types = defs
        .buildings
        .values()
        .map(|building| BuildingTypeInfo {
            id: building.id.clone(),
            name: defs.labels.get(&building.id).cloned(),
            group_id: building.group.clone(),
            city_type: building.city_type.clone(),
        })
        .collect();
    let building_groups = defs
        .building_groups
        .values()
        .map(|group| BuildingGroupInfo {
            id: group.id.clone(),
            name: defs.labels.get(&group.id).cloned(),
            category: group.category.clone(),
            land_usage: group.land_usage.clone(),
            always_possible: group.always_possible,
            default_building: group.default_building.clone(),
            parent_group: group.parent_group.clone(),
        })
        .collect();
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
        buildings_without_orders: world
            .buildings
            .iter()
            .filter(|building| !building.has_orders(defs))
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
        states: detail.states,
        state_goods: detail.state_goods,
        buildings: detail.buildings,
        building_types,
        building_groups,
        state_pops: detail.state_pops,
        state_qualifications: detail.state_qualifications,
        state_needs: detail.state_needs,
        inputs,
        residual,
        status,
        limitations: LIMITATIONS.iter().map(|s| (*s).to_string()).collect(),
    }
}

struct DetailRows {
    states: Vec<StateInfo>,
    state_goods: Vec<StateGood>,
    buildings: Vec<BuildingEconomics>,
    state_pops: Vec<StatePop>,
    state_qualifications: Vec<StateQualification>,
    state_needs: Vec<StateNeed>,
}

fn detail_rows(world: &World, defs: &GameDefs, goods: &[GoodPrice]) -> DetailRows {
    let mut prices = GoodsVec::zeros(defs.goods_order.len());
    let mut base_prices = GoodsVec::zeros(defs.goods_order.len());
    let mut sell_orders = GoodsVec::zeros(defs.goods_order.len());
    for good in goods {
        if let Some(idx) = defs.index_of(&good.id) {
            prices[idx] = good.price;
            base_prices[idx] = good.base;
            sell_orders[idx] = good.sell;
        }
    }
    let rows = goods
        .iter()
        .filter_map(|good| Some((defs.index_of(&good.id)?, good)))
        .collect::<BTreeMap<_, _>>();
    let frozen_sell = world.frozen_sell.aligned(defs.goods_order.len());
    let mut state_buy = BTreeMap::<(u32, GoodIdx), f64>::new();
    let mut state_sell = BTreeMap::<(u32, GoodIdx), f64>::new();

    for state in &world.states {
        let pops = world
            .pops
            .iter()
            .filter(|pop| pop.state == Some(state.id))
            .cloned()
            .collect::<Vec<_>>();
        for (good, quantity) in
            consumption(&pops, &prices, &base_prices, defs, &frozen_sell).iter_indexed()
        {
            *state_buy.entry((state.id, good)).or_default() += quantity;
        }
    }

    let employees_by_building = building_employees(world, defs);

    let mut buildings = Vec::new();
    for building in &world.buildings {
        let (input_qty, output_qty) = building.goods_io(defs);
        let inputs = priced_flows(input_qty, &prices, defs, building.state, &mut state_buy);
        let outputs = priced_flows(output_qty, &prices, defs, building.state, &mut state_sell);
        let cost = inputs.iter().map(|flow| flow.value).sum::<f64>();
        let revenue = outputs.iter().map(|flow| flow.value).sum::<f64>();
        let short_inputs = inputs
            .iter()
            .filter(|flow| {
                defs.index_of(&flow.good_id)
                    .and_then(|idx| rows.get(&idx).copied())
                    .is_none_or(|row| {
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
            employees: employees_by_building
                .get(&building.id)
                .cloned()
                .unwrap_or_default(),
        });
    }

    for trade in &world.state_trade {
        if trade.quantity > 0.0 {
            *state_sell.entry((trade.state, trade.good)).or_default() += trade.quantity;
        } else if trade.quantity < 0.0 {
            *state_buy.entry((trade.state, trade.good)).or_default() -= trade.quantity;
        }
    }

    let state_goods = world
        .states
        .iter()
        .flat_map(|state| rows.iter().map(move |(&idx, row)| (state, idx, *row)))
        .filter_map(|(state, idx, row)| {
            let good_id = defs.good_by_index(idx)?.to_string();
            let buy = state_buy.get(&(state.id, idx)).copied().unwrap_or(0.0);
            let sell = state_sell.get(&(state.id, idx)).copied().unwrap_or(0.0);
            let state_price = price(row.base, buy, sell, defs.price_range.max(0.0));
            let market_access = market_access(state.infrastructure, state.infrastructure_usage);
            let effective_mapi = 0.75 * market_access;
            Some(StateGood {
                state_id: state.id,
                good_id,
                buy,
                sell,
                price: local_price(effective_mapi, row.price, state_price),
                market_price: row.price,
                state_price,
                market_access,
                effective_mapi,
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
            region_name: state
                .region
                .as_ref()
                .and_then(|id| defs.labels.get(id))
                .cloned(),
            country_id: state.country,
            market_id: state.market,
            arable_land: state.arable_land,
            infrastructure: state.infrastructure,
            infrastructure_usage: state.infrastructure_usage,
        })
        .collect();
    let state_pops = collapsed_state_pops(world, defs, &prices, &base_prices, &sell_orders);
    let state_needs = aggregate_state_needs(&state_pops);
    let state_qualifications = state_qualification_rows(world, defs, &employees_by_building);
    DetailRows {
        states,
        state_goods,
        buildings,
        state_pops,
        state_qualifications,
        state_needs,
    }
}

fn profession_counts(map: &BTreeMap<String, f64>, defs: &GameDefs) -> Vec<ProfessionCount> {
    map.iter()
        .filter(|(_, count)| **count > 0.0)
        .map(|(id, count)| ProfessionCount {
            profession_id: id.clone(),
            profession_name: defs.labels.get(id).cloned(),
            count: *count,
        })
        .collect()
}

fn building_employees(world: &World, defs: &GameDefs) -> BTreeMap<u32, Vec<ProfessionCount>> {
    let mut counts: BTreeMap<u32, BTreeMap<String, f64>> = BTreeMap::new();
    for pop in &world.state_pops {
        let Some(building_id) = pop.workplace_id else {
            continue;
        };
        let Some(profession) = pop.profession.as_ref() else {
            continue;
        };
        let workforce = pop.workforce.unwrap_or(0.0);
        if workforce <= 0.0 {
            continue;
        }
        *counts
            .entry(building_id)
            .or_default()
            .entry(profession.clone())
            .or_default() += workforce;
    }
    counts
        .into_iter()
        .map(|(building_id, by_prof)| (building_id, profession_counts(&by_prof, defs)))
        .collect()
}

type PopGroupKey = (u32, Option<String>, Option<String>, Option<i32>);

fn collapsed_state_pops(
    world: &World,
    defs: &GameDefs,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    sell_orders: &GoodsVec,
) -> Vec<StatePop> {
    let source: Vec<WorldStatePop> = if world.state_pops.is_empty() {
        world
            .pops
            .iter()
            .enumerate()
            .filter_map(|(index, pop)| {
                Some(WorldStatePop {
                    id: u32::try_from(index).ok()?,
                    state: pop.state,
                    demand_size: Some(pop.size),
                    workforce: Some(pop.size),
                    dependents: Some(0.0),
                    wealth: Some(i32::from(pop.wealth)),
                    wages: Some(pop.wages),
                    culture: pop.culture.clone(),
                    profession: pop.profession.clone(),
                    literate: None,
                    workplace_id: None,
                    qualifications: BTreeMap::new(),
                })
            })
            .collect()
    } else {
        world.state_pops.clone()
    };

    let mut groups: BTreeMap<PopGroupKey, WorldStatePop> = BTreeMap::new();
    for pop in source {
        let Some(state_id) = pop.state else {
            continue;
        };
        let key = (
            state_id,
            pop.profession.clone(),
            pop.culture.clone(),
            pop.wealth,
        );
        groups
            .entry(key)
            .and_modify(|existing| {
                existing.demand_size =
                    Some(existing.demand_size.unwrap_or(0.0) + pop.demand_size.unwrap_or(0.0));
                existing.workforce =
                    Some(existing.workforce.unwrap_or(0.0) + pop.workforce.unwrap_or(0.0));
                existing.dependents =
                    Some(existing.dependents.unwrap_or(0.0) + pop.dependents.unwrap_or(0.0));
                existing.literate = match (existing.literate, pop.literate) {
                    (Some(left), Some(right)) => Some(left + right),
                    (Some(left), None) => Some(left),
                    (None, Some(right)) => Some(right),
                    (None, None) => None,
                };
                if existing.workplace_id != pop.workplace_id {
                    existing.workplace_id = None;
                }
                for (profession, count) in &pop.qualifications {
                    *existing
                        .qualifications
                        .entry(profession.clone())
                        .or_default() += *count;
                }
            })
            .or_insert(pop);
    }

    groups
        .into_values()
        .map(|pop| {
            let needs = pop_needs_for(&pop, defs, prices, base_prices, sell_orders);
            StatePop {
                id: Some(pop.id),
                state_id: pop.state.unwrap_or(0),
                profession_id: pop.profession.clone(),
                profession_name: pop
                    .profession
                    .as_ref()
                    .and_then(|id| defs.labels.get(id))
                    .cloned(),
                demand_size: pop.demand_size,
                workforce: pop.workforce,
                dependents: pop.dependents,
                wealth: pop.wealth,
                culture_id: pop.culture.clone(),
                culture_name: pop
                    .culture
                    .as_ref()
                    .and_then(|id| defs.labels.get(id))
                    .cloned(),
                literate: pop.literate,
                workplace_id: pop.workplace_id,
                qualifications: profession_counts(&pop.qualifications, defs),
                needs,
            }
        })
        .collect()
}

fn pop_needs_for(
    pop: &WorldStatePop,
    defs: &GameDefs,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    sell_orders: &GoodsVec,
) -> Vec<PopNeedBasket> {
    let Some(size) = pop.demand_size.filter(|size| *size > 0.0) else {
        return Vec::new();
    };
    let Some(wealth) = pop.wealth else {
        return Vec::new();
    };
    let Ok(wealth) = u8::try_from(wealth.clamp(1, 99)) else {
        return Vec::new();
    };
    let world_pop = WorldPop {
        state: pop.state,
        size,
        wealth,
        wages: pop.wages.filter(|wages| *wages > 0.0).unwrap_or(0.0),
        culture: pop.culture.clone(),
        profession: pop.profession.clone(),
    };
    pop_need_baskets(&world_pop, prices, base_prices, defs, sell_orders)
        .into_iter()
        .filter_map(|basket| {
            let need_id = defs.need_by_index(basket.need_idx)?.id.clone();
            Some(PopNeedBasket {
                need_name: defs.labels.get(&need_id).cloned(),
                need_id,
                package_value: basket.package_value,
                goods: basket
                    .goods
                    .into_iter()
                    .filter_map(|(idx, quantity)| {
                        let good_id = defs.good_by_index(idx)?.to_string();
                        Some(GoodFlow {
                            value: quantity * prices[idx],
                            good_id,
                            quantity,
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

fn aggregate_state_needs(pops: &[StatePop]) -> Vec<StateNeed> {
    let mut by_state: BTreeMap<(u32, String), StateNeed> = BTreeMap::new();
    for pop in pops {
        for need in &pop.needs {
            let entry = by_state
                .entry((pop.state_id, need.need_id.clone()))
                .or_insert_with(|| StateNeed {
                    state_id: pop.state_id,
                    need_id: need.need_id.clone(),
                    need_name: need.need_name.clone(),
                    package_value: 0.0,
                    goods: Vec::new(),
                });
            entry.package_value += need.package_value;
            for flow in &need.goods {
                if let Some(existing) = entry
                    .goods
                    .iter_mut()
                    .find(|row| row.good_id == flow.good_id)
                {
                    existing.quantity += flow.quantity;
                    existing.value += flow.value;
                } else {
                    entry.goods.push(flow.clone());
                }
            }
        }
    }
    by_state.into_values().collect()
}

fn state_qualification_rows(
    world: &World,
    defs: &GameDefs,
    employees_by_building: &BTreeMap<u32, Vec<ProfessionCount>>,
) -> Vec<StateQualification> {
    let mut jobs_by_state: BTreeMap<(u32, String), f64> = BTreeMap::new();
    let building_state: BTreeMap<u32, Option<u32>> = world
        .buildings
        .iter()
        .map(|building| (building.id, building.state))
        .collect();
    for (building_id, employees) in employees_by_building {
        let Some(Some(state_id)) = building_state.get(building_id) else {
            continue;
        };
        for employee in employees {
            *jobs_by_state
                .entry((*state_id, employee.profession_id.clone()))
                .or_default() += employee.count;
        }
    }

    let mut employed_by_state: BTreeMap<(u32, String), f64> = BTreeMap::new();
    let mut qualified_from_pops: BTreeMap<(u32, String), f64> = BTreeMap::new();
    for pop in &world.state_pops {
        let Some(state_id) = pop.state else {
            continue;
        };
        if let Some(profession) = &pop.profession {
            let workforce = pop.workforce.unwrap_or(0.0);
            if pop.workplace_id.is_some() && workforce > 0.0 {
                *employed_by_state
                    .entry((state_id, profession.clone()))
                    .or_default() += workforce;
            }
            if workforce > 0.0 {
                *qualified_from_pops
                    .entry((state_id, profession.clone()))
                    .or_default() += workforce;
            }
        }
        for (profession, count) in &pop.qualifications {
            *qualified_from_pops
                .entry((state_id, profession.clone()))
                .or_default() += *count;
        }
    }

    let mut rows = Vec::new();
    for state in &world.states {
        let mut professions: BTreeMap<String, ()> = BTreeMap::new();
        for key in state.qualifications.keys() {
            professions.insert(key.clone(), ());
        }
        for key in state.employable_qualifications.keys() {
            professions.insert(key.clone(), ());
        }
        for key in state.workforce_by_type.keys() {
            professions.insert(key.clone(), ());
        }
        for (state_id, profession) in employed_by_state.keys().chain(jobs_by_state.keys()) {
            if *state_id == state.id {
                professions.insert(profession.clone(), ());
            }
        }
        for (state_id, profession) in qualified_from_pops.keys() {
            if *state_id == state.id {
                professions.insert(profession.clone(), ());
            }
        }
        for profession_id in professions.keys() {
            let qualified = state
                .qualifications
                .get(profession_id)
                .copied()
                .unwrap_or_else(|| {
                    qualified_from_pops
                        .get(&(state.id, profession_id.clone()))
                        .copied()
                        .unwrap_or(0.0)
                });
            let employable = state.employable_qualifications.get(profession_id).copied();
            let employed = employed_by_state
                .get(&(state.id, profession_id.clone()))
                .copied()
                .or_else(|| state.workforce_by_type.get(profession_id).copied())
                .unwrap_or(0.0);
            let jobs = jobs_by_state
                .get(&(state.id, profession_id.clone()))
                .copied()
                .unwrap_or(employed);
            let stock = employable.unwrap_or(qualified);
            rows.push(StateQualification {
                state_id: state.id,
                profession_name: defs.labels.get(profession_id).cloned(),
                profession_id: profession_id.clone(),
                qualified,
                employable,
                employed,
                jobs,
                shortage: (jobs - stock).max(0.0),
                monthly_change: None,
            });
        }
    }
    rows
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
    quantities: GoodsVec,
    prices: &GoodsVec,
    defs: &GameDefs,
    state: Option<u32>,
    state_side: &mut BTreeMap<(u32, GoodIdx), f64>,
) -> Vec<GoodFlow> {
    quantities
        .iter_indexed()
        .filter(|(_, quantity)| quantity.abs() > crate::ORDER_EPS)
        .filter_map(|(good, quantity)| {
            if let Some(state_id) = state {
                *state_side.entry((state_id, good)).or_default() += quantity;
            }
            Some(GoodFlow {
                value: prices[good] * quantity,
                good_id: defs.good_by_index(good)?.to_string(),
                quantity,
            })
        })
        .collect()
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

/// Split pops into wage-sensitive vs frozen-wealth; precompute the constant buy.
fn split_pop_buy(
    world: &World,
    defs: &GameDefs,
    base_prices: &GoodsVec,
    frozen_sell: &GoodsVec,
    pop_access: &[f64],
) -> (Vec<usize>, GoodsVec) {
    let mut wage_pop_idxs = Vec::new();
    let mut frozen_pop_buy = GoodsVec::zeros(defs.goods_order.len());
    for (i, pop) in world.pops.iter().enumerate() {
        if pop.wages > 0.0 {
            wage_pop_idxs.push(i);
        } else {
            // Prices are irrelevant for wages ≤ 0 (wealth is frozen).
            add_pop_consumption_scaled(
                &mut frozen_pop_buy,
                pop,
                base_prices,
                base_prices,
                defs,
                frozen_sell,
                pop_access.get(i).copied().unwrap_or(1.0),
            );
        }
    }
    (wage_pop_idxs, frozen_pop_buy)
}

#[derive(Clone)]
struct PriceResidual<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    goods: &'a [GoodIdx],
    bases: &'a [f64],
    base_prices: GoodsVec,
    price_range: f64,
    lower: Vec<f64>,
    upper: Vec<f64>,
    frozen_buy: GoodsVec,
    frozen_sell: GoodsVec,
    wage_pop_idxs: Vec<usize>,
    frozen_pop_buy: GoodsVec,
    pop_access: Vec<f64>,
}

impl PriceResidual<'_> {
    fn prices_from_rel(&self, rel: &[f64]) -> GoodsVec {
        let mut prices = self.base_prices.clone();
        for (&good, (&base, &r)) in self.goods.iter().zip(self.bases.iter().zip(rel)) {
            prices[good] = base * r;
        }
        prices
    }

    fn pop_buy_at(&self, prices: &GoodsVec) -> GoodsVec {
        let mut buy = self.frozen_pop_buy.clone();
        for &i in &self.wage_pop_idxs {
            add_pop_consumption_scaled(
                &mut buy,
                &self.world.pops[i],
                prices,
                &self.base_prices,
                self.defs,
                &self.frozen_sell,
                self.pop_access.get(i).copied().unwrap_or(1.0),
            );
        }
        buy
    }

    fn formula_rel(&self, rel: &[f64]) -> Vec<f64> {
        let prices = self.prices_from_rel(rel);
        let pop_buy = self.pop_buy_at(&prices);
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
        let pop_buy = self.pop_buy_at(&prices);
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
