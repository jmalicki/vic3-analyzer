//! Bound-constrained NLS (`basin::Trf`) plus successive-substitution warm start.
//!
//! The NLS / settle loop is ~1 ms on a late autosave; building shops is ~80 ms.
//! [`finished`] still builds the full UI payload (compact pop rows, need baskets,
//! qualifications) and dominates (~250 ms). CLI `prices` times the same work as
//! wasm `load_analysis`. Do not add a goods-only solve because the default table
//! omits pops.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;

use basin::{
    BoxConstraints, CostFunction, DenseMatrix, Executor, Jacobian, Residual, TerminationReason, Trf,
};
use vic3_defs::{GameDefs, GoodIdx, GoodsVec, NeedIdx};

use crate::consumption::{
    add_pop_from_units, add_wage_bins, consumption, pop_compact_needs, wage_bins_from_pops,
    NeedShares, UnitBaskets, UnitNeedBaskets, WealthBin,
};
use crate::formula::{effective_mapi, local_price, market_access, price};
use crate::result::{
    BuildingEconomics, BuildingGroupInfo, BuildingTypeInfo, CompactNeed, CompactStatePop,
    CountryInfo, EmitTables, GoodFlow, GoodPrice, MarketInputs, PricesResult, ProfessionCount,
    SolveOpts, SolveStatus, StateGood, StateInfo, StateNeed, StatePopList, StateQualification,
};
use crate::world::{qty_value, World, WorldPop, WorldStatePop};
use crate::LIMITATIONS;

const WARM_START_ALPHA: f64 = 0.5;
const FD_STEP: f64 = 1e-7;
const LOCAL_ITERS: u32 = 16;
const LOCAL_EPS: f64 = 1e-10;

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
/// 4. Pops shop at each state's MAPI-blended local prices; those orders are access-scaled into one whole-save market. Extra MAPI modifiers and overseas constraints are not modeled.
/// 5. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.
pub fn solve(world: &World, defs: &GameDefs, opts: SolveOpts) -> PricesResult {
    let base_prices: GoodsVec = defs
        .goods_order
        .iter()
        .map(|id| defs.base_price(id).unwrap_or(0.0))
        .collect();
    let goods = market_goods(&base_prices);
    if goods.is_empty() {
        return finished(
            world,
            defs,
            Vec::new(),
            0.0,
            SolveStatus::Converged,
            None,
            Vec::new(),
        );
    }

    let bases: Vec<f64> = goods.iter().map(|&idx| base_prices[idx]).collect();
    if bases.iter().any(|b| *b <= 0.0) {
        return finished(
            world,
            defs,
            Vec::new(),
            f64::INFINITY,
            SolveStatus::Failed,
            None,
            Vec::new(),
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

    finished(world, defs, rows, residual, status, Some(&snapshot), rel)
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
    snapshot: Option<&ShopSnapshot>,
    relative: Vec<f64>,
) -> PricesResult {
    let detail = detail_rows(world, defs, &goods, snapshot);
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
        pops: world.pop_count(),
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
        relative,
    }
}

struct DetailRows {
    states: Vec<StateInfo>,
    state_goods: Vec<StateGood>,
    buildings: Vec<BuildingEconomics>,
    state_pops: StatePopList,
    state_qualifications: Vec<StateQualification>,
    state_needs: Vec<StateNeed>,
}

fn detail_rows(
    world: &World,
    defs: &GameDefs,
    goods: &[GoodPrice],
    snapshot: Option<&ShopSnapshot>,
) -> DetailRows {
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
    let shares = NeedShares::from_sell(defs, &sell_orders);
    let units = UnitBaskets::from_shares(defs, &base_prices, &shares);
    let need_units = UnitNeedBaskets::from_shares(defs, &base_prices, &shares);
    let mut state_buy = BTreeMap::<(u32, GoodIdx), f64>::new();
    let mut state_sell = BTreeMap::<(u32, GoodIdx), f64>::new();

    let pops_by_state = snapshot.is_none().then(|| {
        let mut index = BTreeMap::<u32, Vec<WorldPop>>::new();
        for pop in world.iter_pops() {
            if let Some(state) = pop.state {
                index.entry(state).or_default().push(pop);
            }
        }
        index
    });

    for state in &world.states {
        let pop_buy = snapshot
            .and_then(|snap| snap.pop_buy_by_state.get(&state.id))
            .cloned()
            .unwrap_or_else(|| {
                let empty: &[WorldPop] = &[];
                let pops = pops_by_state
                    .as_ref()
                    .and_then(|index| index.get(&state.id))
                    .map(Vec::as_slice)
                    .unwrap_or(empty);
                consumption(pops, &prices, &base_prices, defs, &frozen_sell)
            });
        for (good, quantity) in pop_buy.iter_indexed() {
            if quantity.abs() > crate::ORDER_EPS {
                *state_buy.entry((state.id, good)).or_default() += quantity;
            }
        }
    }

    let employees_by_building = building_employees(world, defs);

    let mut buildings = Vec::new();
    for building in &world.buildings {
        let (input_qty, output_qty) = building.goods_io(defs);
        let local = building
            .state
            .and_then(|state| snapshot.and_then(|snap| snap.local_by_state.get(&state)))
            .unwrap_or(&prices);
        let inputs = priced_flows(input_qty, local, defs, building.state, &mut state_buy);
        let outputs = priced_flows(output_qty, local, defs, building.state, &mut state_sell);
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
            let effective_mapi = effective_mapi(market_access);
            let price = snapshot
                .and_then(|snap| snap.local_by_state.get(&state.id))
                .map(|local| local[idx])
                .unwrap_or_else(|| local_price(effective_mapi, row.price, state_price));
            Some(StateGood {
                state_id: state.id,
                good_id,
                buy,
                sell,
                price,
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
    let tables = EmitTables::from_world(world, defs);
    let compact_pops =
        collapsed_state_pops(world, &prices, &base_prices, &units, &need_units, snapshot);
    let state_needs = aggregate_state_needs(&compact_pops, &tables);
    let state_pops = StatePopList::compact(tables, compact_pops);
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

fn profession_counts<I>(pairs: I, world: &World, defs: &GameDefs) -> Vec<ProfessionCount>
where
    I: IntoIterator<Item = (u16, f64)>,
{
    pairs
        .into_iter()
        .filter(|(_, count)| *count > 0.0)
        .filter_map(|(id, count)| {
            let profession_id = world.name(id)?.to_string();
            Some(ProfessionCount {
                profession_name: defs.labels.get(&profession_id).cloned(),
                profession_id,
                count,
            })
        })
        .collect()
}

fn building_employees(world: &World, defs: &GameDefs) -> BTreeMap<u32, Vec<ProfessionCount>> {
    let mut counts: BTreeMap<u32, BTreeMap<u16, f64>> = BTreeMap::new();
    for pop in &world.state_pops {
        let Some(building_id) = pop.workplace_id else {
            continue;
        };
        let Some(profession) = pop.profession else {
            continue;
        };
        let workforce = pop.workforce.unwrap_or(0.0);
        if workforce <= 0.0 {
            continue;
        }
        *counts
            .entry(building_id)
            .or_default()
            .entry(profession)
            .or_default() += workforce;
    }
    counts
        .into_iter()
        .map(|(building_id, by_prof)| (building_id, profession_counts(by_prof, world, defs)))
        .collect()
}

type GroupKey = (u32, Option<u16>, Option<u16>, Option<i32>);

struct PopGroup {
    id: u32,
    state: u32,
    demand_size: Option<f64>,
    workforce: Option<f64>,
    dependents: Option<f64>,
    wealth: Option<i32>,
    wages: Option<f64>,
    culture: Option<u16>,
    profession: Option<u16>,
    literate: Option<f64>,
    workplace_id: Option<u32>,
    qualifications: Vec<(u16, f64)>,
}

impl PopGroup {
    fn from_state_pop(pop: &WorldStatePop, state: u32) -> Self {
        Self {
            id: pop.id,
            state,
            demand_size: pop.demand_size,
            workforce: pop.workforce,
            dependents: pop.dependents,
            wealth: pop.wealth,
            wages: pop.wages,
            culture: pop.culture,
            profession: pop.profession,
            literate: pop.literate,
            workplace_id: pop.workplace_id,
            qualifications: pop.qualifications.clone(),
        }
    }

    fn from_world_pop(id: u32, pop: &WorldPop, state: u32) -> Self {
        Self {
            id,
            state,
            demand_size: Some(pop.size),
            workforce: Some(pop.size),
            dependents: Some(0.0),
            wealth: Some(i32::from(pop.wealth)),
            wages: Some(pop.wages),
            culture: pop.culture,
            profession: pop.profession,
            literate: None,
            workplace_id: None,
            qualifications: Vec::new(),
        }
    }

    fn add_state_pop(&mut self, pop: &WorldStatePop) {
        self.demand_size = Some(self.demand_size.unwrap_or(0.0) + pop.demand_size.unwrap_or(0.0));
        self.workforce = Some(self.workforce.unwrap_or(0.0) + pop.workforce.unwrap_or(0.0));
        self.dependents = Some(self.dependents.unwrap_or(0.0) + pop.dependents.unwrap_or(0.0));
        self.literate = match (self.literate, pop.literate) {
            (Some(left), Some(right)) => Some(left + right),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        };
        if self.workplace_id != pop.workplace_id {
            self.workplace_id = None;
        }
        add_qty(&mut self.qualifications, &pop.qualifications);
    }

    fn add_world_pop(&mut self, pop: &WorldPop) {
        self.demand_size = Some(self.demand_size.unwrap_or(0.0) + pop.size);
        self.workforce = Some(self.workforce.unwrap_or(0.0) + pop.size);
        if self.workplace_id.is_some() {
            self.workplace_id = None;
        }
    }
}

fn add_qty(dst: &mut Vec<(u16, f64)>, src: &[(u16, f64)]) {
    for &(id, qty) in src {
        if let Some(existing) = dst.iter_mut().find(|(stored, _)| *stored == id) {
            existing.1 += qty;
        } else {
            dst.push((id, qty));
        }
    }
}

fn collapsed_state_pops(
    world: &World,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    need_units: &UnitNeedBaskets,
    snapshot: Option<&ShopSnapshot>,
) -> Vec<CompactStatePop> {
    // HashMap insert is cheaper than BTreeMap on the late-save grouping
    // sample. Sort groups afterward so JSON `state_pops` stays ordered by
    // (state, profession, culture, wealth).
    let mut groups: HashMap<GroupKey, PopGroup> =
        HashMap::with_capacity(world.state_pops.len().max(world.pops.len()));
    if world.state_pops.is_empty() {
        for (index, pop) in world.iter_pops().enumerate() {
            let Some(state_id) = pop.state else {
                continue;
            };
            let key = (
                state_id,
                pop.profession,
                pop.culture,
                Some(i32::from(pop.wealth)),
            );
            groups
                .entry(key)
                .and_modify(|existing| existing.add_world_pop(&pop))
                .or_insert_with(|| {
                    PopGroup::from_world_pop(u32::try_from(index).unwrap_or(0), &pop, state_id)
                });
        }
    } else {
        for pop in &world.state_pops {
            let Some(state_id) = pop.state else {
                continue;
            };
            let key = (state_id, pop.profession, pop.culture, pop.wealth);
            groups
                .entry(key)
                .and_modify(|existing| existing.add_state_pop(pop))
                .or_insert_with(|| PopGroup::from_state_pop(pop, state_id));
        }
    }

    let mut groups: Vec<PopGroup> = groups.into_values().collect();
    groups.sort_unstable_by_key(|pop| (pop.state, pop.profession, pop.culture, pop.wealth));
    groups
        .into_iter()
        .map(|mut pop| {
            pop.qualifications
                .sort_unstable_by_key(|(profession, _)| *profession);
            let local = snapshot
                .and_then(|snap| snap.local_by_state.get(&pop.state))
                .unwrap_or(prices);
            let needs = pop_needs_for(&pop, local, base_prices, units, need_units);
            CompactStatePop {
                id: Some(pop.id),
                state_id: pop.state,
                profession: pop.profession,
                demand_size: pop.demand_size,
                workforce: pop.workforce,
                dependents: pop.dependents,
                wealth: pop.wealth,
                culture: pop.culture,
                literate: pop.literate,
                workplace_id: pop.workplace_id,
                qualifications: pop
                    .qualifications
                    .into_iter()
                    .filter(|(_, count)| *count > 0.0)
                    .collect(),
                needs,
            }
        })
        .collect()
}

fn pop_needs_for(
    pop: &PopGroup,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    need_units: &UnitNeedBaskets,
) -> Vec<CompactNeed> {
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
        state: Some(pop.state),
        size,
        wealth,
        wages: pop.wages.filter(|wages| *wages > 0.0).unwrap_or(0.0),
        culture: None,
        profession: None,
    };
    pop_compact_needs(&world_pop, prices, base_prices, units, need_units)
}

fn add_need_good(dst: &mut Vec<(GoodIdx, f64, f64)>, idx: GoodIdx, quantity: f64, value: f64) {
    if let Some(existing) = dst.iter_mut().find(|(stored, _, _)| *stored == idx) {
        existing.1 += quantity;
        existing.2 += value;
    } else {
        dst.push((idx, quantity, value));
    }
}

struct NeedAgg {
    state_id: u32,
    need_idx: NeedIdx,
    package_value: f64,
    goods: Vec<(GoodIdx, f64, f64)>,
}

fn aggregate_state_needs(pops: &[CompactStatePop], tables: &EmitTables) -> Vec<StateNeed> {
    let mut by_state: HashMap<(u32, NeedIdx), NeedAgg> = HashMap::new();
    for pop in pops {
        for need in &pop.needs {
            let entry = by_state
                .entry((pop.state_id, need.need_idx))
                .or_insert_with(|| NeedAgg {
                    state_id: pop.state_id,
                    need_idx: need.need_idx,
                    package_value: 0.0,
                    goods: Vec::new(),
                });
            entry.package_value += need.package_value;
            for &(idx, quantity, value) in &need.goods {
                add_need_good(&mut entry.goods, idx, quantity, value);
            }
        }
    }

    let mut rows: Vec<NeedAgg> = by_state.into_values().collect();
    rows.sort_unstable_by_key(|row| (row.state_id, row.need_idx));
    rows.into_iter()
        .filter_map(|mut row| {
            let need_id = tables.need(row.need_idx)?.to_string();
            row.goods.sort_unstable_by_key(|(idx, _, _)| *idx);
            Some(StateNeed {
                state_id: row.state_id,
                need_name: tables.label(&need_id).map(str::to_string),
                need_id,
                package_value: row.package_value,
                goods: row
                    .goods
                    .into_iter()
                    .filter_map(|(idx, quantity, value)| {
                        Some(GoodFlow {
                            good_id: tables.good(idx)?.to_string(),
                            quantity,
                            value,
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

fn state_qualification_rows(
    world: &World,
    defs: &GameDefs,
    employees_by_building: &BTreeMap<u32, Vec<ProfessionCount>>,
) -> Vec<StateQualification> {
    let mut jobs_by_state: BTreeMap<(u32, u16), f64> = BTreeMap::new();
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
            let Some(profession) = world.names.id_of(&employee.profession_id) else {
                continue;
            };
            *jobs_by_state.entry((*state_id, profession)).or_default() += employee.count;
        }
    }

    let mut employed_by_state: BTreeMap<(u32, u16), f64> = BTreeMap::new();
    let mut qualified_from_pops: BTreeMap<(u32, u16), f64> = BTreeMap::new();
    for pop in &world.state_pops {
        let Some(state_id) = pop.state else {
            continue;
        };
        if let Some(profession) = pop.profession {
            let workforce = pop.workforce.unwrap_or(0.0);
            if pop.workplace_id.is_some() && workforce > 0.0 {
                *employed_by_state.entry((state_id, profession)).or_default() += workforce;
            }
            if workforce > 0.0 {
                *qualified_from_pops
                    .entry((state_id, profession))
                    .or_default() += workforce;
            }
        }
        for &(profession, count) in &pop.qualifications {
            *qualified_from_pops
                .entry((state_id, profession))
                .or_default() += count;
        }
    }

    let mut rows = Vec::new();
    for state in &world.states {
        let mut professions: BTreeMap<u16, ()> = BTreeMap::new();
        for &(id, _) in &state.qualifications {
            professions.insert(id, ());
        }
        for &(id, _) in &state.employable_qualifications {
            professions.insert(id, ());
        }
        for &(id, _) in &state.workforce_by_type {
            professions.insert(id, ());
        }
        for &(state_id, profession) in employed_by_state.keys() {
            if state_id == state.id {
                professions.insert(profession, ());
            }
        }
        for &(state_id, profession) in jobs_by_state.keys() {
            if state_id == state.id {
                professions.insert(profession, ());
            }
        }
        for &(state_id, profession) in qualified_from_pops.keys() {
            if state_id == state.id {
                professions.insert(profession, ());
            }
        }
        for profession in professions.keys().copied() {
            let Some(profession_id) = world.name(profession).map(str::to_string) else {
                continue;
            };
            let qualified = qty_value(&state.qualifications, profession).unwrap_or_else(|| {
                qualified_from_pops
                    .get(&(state.id, profession))
                    .copied()
                    .unwrap_or(0.0)
            });
            let employable = qty_value(&state.employable_qualifications, profession);
            let employed = employed_by_state
                .get(&(state.id, profession))
                .copied()
                .or_else(|| qty_value(&state.workforce_by_type, profession))
                .unwrap_or(0.0);
            let jobs = jobs_by_state
                .get(&(state.id, profession))
                .copied()
                .unwrap_or(employed);
            let stock = employable.unwrap_or(qualified);
            rows.push(StateQualification {
                state_id: state.id,
                profession_name: defs.labels.get(&profession_id).cloned(),
                profession_id,
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
struct ShopSnapshot {
    world_pop_buy: GoodsVec,
    local_by_state: BTreeMap<u32, GoodsVec>,
    pop_buy_by_state: BTreeMap<u32, GoodsVec>,
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
