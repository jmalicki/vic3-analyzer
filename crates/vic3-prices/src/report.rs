//! Build the full [`PricesResult`] UI/SQL payload from a price solve.
//!
//! The NLS lives in [`crate::solve`]; this module only packages goods prices
//! into countries, state locals, building economics, pops, and catalog rows.
//! Planning and other hot callers should use [`crate::solve::equilibrate`] and
//! never call [`report`].

use std::collections::{BTreeMap, HashMap};

use vic3_defs::{GameDefs, GoodId, GoodsVec, NeedId};

use crate::consumption::{
    consumption, pop_compact_needs, NeedShares, UnitBaskets, UnitNeedBaskets,
};
use crate::formula::{effective_mapi, local_price, market_access, price};
use crate::result::{
    BuildingEconomics, BuildingGroupInfo, BuildingTypeInfo, CompactNeed, CompactStatePop,
    CountryInfo, EmitTables, GoodFlow, GoodPrice, MarketInputs, PricesResult, ProfessionCount,
    SolveOutcome, StateGood, StateInfo, StateNeed, StatePopList, StateQualification,
};
use crate::solve::ShopSnapshot;
use crate::world::{qty_value, World, WorldPop, WorldStatePop};
use crate::LIMITATIONS;

/// Package a compact [`SolveOutcome`] into the full public [`PricesResult`].
///
/// Re-derives local prices / pop tables from `outcome.goods` (no settle snapshot).
/// Prefer [`report_from_solve`] on the CLI path when a settle snapshot is available.
pub fn report(world: &World, defs: &GameDefs, outcome: &SolveOutcome) -> PricesResult {
    report_from_solve(world, defs, outcome, None)
}

/// Package solve outputs using a settle [`ShopSnapshot`] when available.
pub(crate) fn report_from_solve(
    world: &World,
    defs: &GameDefs,
    outcome: &SolveOutcome,
    snapshot: Option<&ShopSnapshot>,
) -> PricesResult {
    let detail = detail_rows(world, defs, &outcome.goods, snapshot);
    let countries = country_rows(world, defs);
    let building_types = building_type_infos(defs);
    let building_groups = building_group_infos(defs);
    let inputs = market_inputs(world, defs, &outcome.goods);
    PricesResult {
        scope: "whole_save_synthetic".to_string(),
        goods: outcome.goods.clone(),
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
        residual: outcome.residual,
        status: outcome.status,
        limitations: solve_limitations(),
        relative: outcome.relative.clone(),
    }
}

/// Def catalog rows for [`PricesResult::building_types`].
fn building_type_infos(defs: &GameDefs) -> Vec<BuildingTypeInfo> {
    defs.buildings
        .values()
        .map(|building| BuildingTypeInfo {
            id: building.id.clone(),
            name: defs.labels.get(&building.id).cloned(),
            group_id: building.group.clone(),
            city_type: building.city_type.clone(),
        })
        .collect()
}

/// Def catalog rows for [`PricesResult::building_groups`].
fn building_group_infos(defs: &GameDefs) -> Vec<BuildingGroupInfo> {
    defs.building_groups
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
        .collect()
}

/// Input / skip counters for [`PricesResult::inputs`].
fn market_inputs(world: &World, defs: &GameDefs, goods: &[GoodPrice]) -> MarketInputs {
    MarketInputs {
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
    }
}

fn solve_limitations() -> Vec<String> {
    LIMITATIONS.iter().map(|s| (*s).to_string()).collect()
}

struct DetailRows {
    states: Vec<StateInfo>,
    state_goods: Vec<StateGood>,
    buildings: Vec<BuildingEconomics>,
    state_pops: StatePopList,
    state_qualifications: Vec<StateQualification>,
    state_needs: Vec<StateNeed>,
}

/// Assemble per-state / building / pop detail tables for [`PricesResult`].
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
        if let Some(idx) = defs.index_of(&good.name) {
            prices[idx] = good.price;
            base_prices[idx] = good.base;
            sell_orders[idx] = good.sell;
        }
    }
    let rows = goods
        .iter()
        .filter_map(|good| Some((defs.index_of(&good.name)?, good)))
        .collect::<BTreeMap<_, _>>();
    let frozen_sell = world.frozen_sell.aligned(defs.goods_order.len());
    let shares = NeedShares::from_sell(defs, &sell_orders);
    let units = UnitBaskets::from_shares(defs, &base_prices, &shares);
    let need_units = UnitNeedBaskets::from_shares(defs, &base_prices, &shares);
    let mut state_buy = BTreeMap::<(u32, GoodId), f64>::new();
    let mut state_sell = BTreeMap::<(u32, GoodId), f64>::new();

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
                defs.index_of(&flow.name)
                    .and_then(|idx| rows.get(&idx).copied())
                    .is_none_or(|row| {
                        row.sell <= crate::ORDER_EPS
                            || row.price
                                >= row.base * (1.0 + defs.price_range.max(0.0)) - crate::ORDER_EPS
                    })
            })
            .map(|flow| flow.name.clone())
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
                name: good_id,
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
    let mut states: Vec<StateInfo> = world
        .states
        .iter()
        .map(|state| StateInfo {
            id: state.id,
            region_name: state.region.clone(),
            region_label: state
                .region
                .as_ref()
                .map(|id| crate::label::script_label(defs, id)),
            state_label: state
                .region
                .as_ref()
                .map(|id| crate::label::script_label(defs, id)),
            country_id: state.country,
            market_id: state.market,
            arable_land: state.arable_land,
            infrastructure: state.infrastructure,
            infrastructure_usage: state.infrastructure_usage,
        })
        .collect();
    let tags_by_country: HashMap<u32, &str> = world
        .countries
        .iter()
        .map(|c| (c.id, c.tag.as_str()))
        .collect();
    crate::label::apply_split_state_demonyms(&mut states, &tags_by_country, defs);
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
            let profession_name = world.name(id)?.to_string();
            Some(ProfessionCount {
                name: profession_name.clone(),
                label: defs.labels.get(&profession_name).cloned(),
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

fn add_need_good(dst: &mut Vec<(GoodId, f64, f64)>, idx: GoodId, quantity: f64, value: f64) {
    if let Some(existing) = dst.iter_mut().find(|(stored, _, _)| *stored == idx) {
        existing.1 += quantity;
        existing.2 += value;
    } else {
        dst.push((idx, quantity, value));
    }
}

struct NeedAgg {
    state_id: u32,
    need_id: NeedId,
    package_value: f64,
    goods: Vec<(GoodId, f64, f64)>,
}

fn aggregate_state_needs(pops: &[CompactStatePop], tables: &EmitTables) -> Vec<StateNeed> {
    let mut by_state: HashMap<(u32, NeedId), NeedAgg> = HashMap::new();
    for pop in pops {
        for need in &pop.needs {
            let entry = by_state
                .entry((pop.state_id, need.need_id))
                .or_insert_with(|| NeedAgg {
                    state_id: pop.state_id,
                    need_id: need.need_id,
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
    rows.sort_unstable_by_key(|row| (row.state_id, row.need_id));
    rows.into_iter()
        .filter_map(|mut row| {
            let need_name = tables.need(row.need_id)?.to_string();
            row.goods.sort_unstable_by_key(|(idx, _, _)| *idx);
            Some(StateNeed {
                state_id: row.state_id,
                name: need_name.clone(),
                label: tables.label(&need_name).map(str::to_string),
                package_value: row.package_value,
                goods: row
                    .goods
                    .into_iter()
                    .filter_map(|(idx, quantity, value)| {
                        Some(GoodFlow {
                            name: tables.good(idx)?.to_string(),
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
            let Some(profession) = world.names.id_of(&employee.name) else {
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
                name: profession_id.clone(),
                label: defs.labels.get(&profession_id).cloned(),
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
                country_name: country.tag.clone(),
                country_label: defs.labels.get(&country.tag).cloned(),
                adjective: crate::label::country_adjective(defs, &country.tag),
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
    state_side: &mut BTreeMap<(u32, GoodId), f64>,
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
                name: defs.good_by_index(good)?.to_string(),
                quantity,
            })
        })
        .collect()
}

/// Per-building revenue at solved local prices (planning GDP; no full I/O rows).
///
/// World-based twin of [`building_revenues_from_cache`]. Kept for cold-path /
/// parity callers that still hold a [`World`].
///
/// * `world` — building list and states.
/// * `defs` — good index lookup for `goods` prices.
/// * `goods` — solved market good prices.
/// * `snapshot` — optional per-state local prices from settle; falls back to market.
#[allow(dead_code)]
pub fn building_revenues(
    world: &World,
    defs: &GameDefs,
    goods: &[GoodPrice],
    snapshot: Option<&ShopSnapshot>,
) -> Vec<crate::result::BuildingRevenue> {
    let mut prices = GoodsVec::zeros(defs.goods_order.len());
    for good in goods {
        if let Some(idx) = defs.index_of(&good.name) {
            prices[idx] = good.price;
        }
    }
    world
        .buildings
        .iter()
        .map(|building| {
            let (_, output_qty) = building.goods_io(defs);
            let local = building
                .state
                .and_then(|state| snapshot.and_then(|snap| snap.local_by_state.get(&state)))
                .unwrap_or(&prices);
            let revenue = output_qty
                .iter_indexed()
                .filter(|(_, quantity)| quantity.abs() > crate::ORDER_EPS)
                .map(|(good, quantity)| local[good] * quantity)
                .sum::<f64>();
            crate::result::BuildingRevenue {
                state_id: building.state,
                revenue,
            }
        })
        .collect()
}

/// Same as [`building_revenues`] but reads IO from a [`crate::ShopCache`] (no World).
///
/// * `cache` — patched or baseline shops with `buildings` IO rows.
/// * `defs` — good index lookup.
/// * `goods` — solved market prices.
/// * `snapshot` — optional local prices by state.
pub fn building_revenues_from_cache(
    cache: &crate::ShopCache,
    defs: &GameDefs,
    goods: &[GoodPrice],
    snapshot: Option<&ShopSnapshot>,
) -> Vec<crate::result::BuildingRevenue> {
    let mut prices = GoodsVec::zeros(defs.goods_order.len());
    for good in goods {
        if let Some(idx) = defs.index_of(&good.name) {
            prices[idx] = good.price;
        }
    }
    cache
        .buildings
        .iter()
        .map(|building| {
            let local = building
                .state_id
                .and_then(|state| snapshot.and_then(|snap| snap.local_by_state.get(&state)))
                .unwrap_or(&prices);
            let revenue = building
                .outputs
                .iter_indexed()
                .filter(|(_, quantity)| quantity.abs() > crate::ORDER_EPS)
                .map(|(good, quantity)| local[good] * quantity)
                .sum::<f64>();
            crate::result::BuildingRevenue {
                state_id: building.state_id,
                revenue,
            }
        })
        .collect()
}
