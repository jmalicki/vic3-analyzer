//! Incremental shop / market-order cache for hot [`crate::equilibrate`] re-solves.
//!
//! # Role
//!
//! Cold solves build this once from a [`World`]. Planning keeps a **baseline**
//! cache on the economy context and, inside a state transition, **patches**
//! building IO from planning deltas instead of rescanning every building.
//!
//! Search / A* never sees this type — only apply/bookkeeping does.
//!
//! # Invariant
//!
//! `patch` after the same building edits must match `from_world` on the
//! projected world (see unit test below).

use std::collections::BTreeMap;
use std::sync::Arc;

use vic3_defs::{GameDefs, GoodsVec};

use crate::consumption::{
    add_pop_from_units, wage_bins_from_pops, NeedShares, UnitBaskets, WealthBin,
};
use crate::formula::{effective_mapi, market_access};
use crate::world::{World, WorldPop};

/// Per-state frozen non-pop orders and pop settle inputs for local price loops.
///
/// Held behind [`Arc`] so a planning edge can `make_mut` one state without
/// deep-copying every shop.
#[derive(Clone, Debug)]
pub struct StateShop {
    /// Save state id.
    pub id: u32,
    /// Infrastructure market access in `[0, 1]`.
    pub access: f64,
    /// `BASE_MAPI * access` — blend weight for local vs market price.
    pub mapi: f64,
    /// Building + trade buy orders attributed to this state (unscaled by access).
    pub frozen_buy: GoodsVec,
    /// Building + trade sell orders attributed to this state (unscaled by access).
    pub frozen_sell: GoodsVec,
    /// Non-wage pop buy at base prices (wage pops live in [`Self::wage_bins`]).
    pub frozen_pop_buy: GoodsVec,
    /// Wage-earning pops binned for price-sensitive consumption.
    pub(crate) wage_bins: Vec<WealthBin>,
}

/// Per-building IO snapshot used for GDP / revenue after a cached solve.
#[derive(Clone, Debug)]
pub struct BuildingIo {
    /// Building instance id in the world.
    pub building_id: u32,
    /// Owning state, if any (`None` = global / 100% access).
    pub state_id: Option<u32>,
    /// Input goods quantities (same basis as [`crate::world::WorldBuilding::goods_io`]).
    pub inputs: GoodsVec,
    /// Output goods quantities.
    pub outputs: GoodsVec,
}

/// Derived shop + market orders so NLS can run without rescanning the world.
///
/// Built by [`ShopCache::from_world`]. Planning clones a baseline, calls
/// [`ShopCache::patch_building_io`] for touched buildings, then
/// [`crate::equilibrate_cached`].
#[derive(Clone, Debug)]
pub struct ShopCache {
    /// One shop per state that has pops, buildings, trade, or infra.
    pub shops: Vec<Arc<StateShop>>,
    /// Access-scaled market frozen buy (buildings + trade + `world.frozen_*`).
    pub frozen_buy: GoodsVec,
    /// Access-scaled market frozen sell.
    pub frozen_sell: GoodsVec,
    /// Pop consumption unit baskets from the current sell mix.
    pub(crate) units: UnitBaskets,
    /// Def base prices aligned to `goods_order`.
    pub(crate) base_prices: GoodsVec,
    /// State id → market access used when scaling orders into the market totals.
    pub(crate) access_by_state: BTreeMap<u32, f64>,
    /// Wage bins for pops with no state.
    pub(crate) stateless_wage_bins: Vec<WealthBin>,
    /// Access-scaled frozen pop buy (stateless + each shop’s frozen pops).
    pub(crate) frozen_pop_buy: GoodsVec,
    /// Building IO rows mirroring the world at build/patch time (for revenues).
    pub buildings: Vec<Arc<BuildingIo>>,
}

impl ShopCache {
    /// Full rebuild: same inputs as today’s cold `equilibrate` setup.
    ///
    /// * `world` — save/projected world (buildings, pops, trade, infra).
    /// * `defs` — goods order, base prices, PM recipes for `goods_io`.
    pub fn from_world(world: &World, defs: &GameDefs) -> Self {
        // 1) Base prices vector aligned to defs.goods_order.
        let base_prices: GoodsVec = defs
            .goods_order
            .iter()
            .map(|id| defs.base_price(id).unwrap_or(0.0))
            .collect();
        // 2) Per-state market access from infrastructure / usage.
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
        // 3) Market-level frozen buy/sell (access-scaled buildings + trade).
        let (frozen_buy, frozen_sell) = access_scaled_non_pop_orders(world, defs, &access_by_state);
        let n_goods = defs.goods_order.len();
        // 4) Pop basket shape from the sell mix (need shares → unit baskets).
        let shares = NeedShares::from_sell(defs, &frozen_sell);
        let units = UnitBaskets::from_shares(defs, &base_prices, &shares);
        // 5) Per-state shops (local orders + pop bins).
        let shops = state_shops(world, defs, &access_by_state, n_goods, &base_prices, &units);
        // 6) Fold state frozen pops into market pop buy; collect stateless wage bins.
        let (stateless_wage_bins, frozen_pop_buy) =
            split_pop_buy(world, n_goods, &base_prices, &units, &shops);
        // 7) Snapshot building IO for GDP/revenue without keeping the full World.
        let buildings = world
            .buildings
            .iter()
            .map(|building| {
                let (inputs, outputs) = building.goods_io(defs);
                Arc::new(BuildingIo {
                    building_id: building.id,
                    state_id: building.state,
                    inputs,
                    outputs,
                })
            })
            .collect();
        Self {
            shops,
            frozen_buy,
            frozen_sell,
            units,
            base_prices,
            access_by_state,
            stateless_wage_bins,
            frozen_pop_buy,
            buildings,
        }
    }

    /// Subtract old building IO and add new IO on the state’s shop and market totals.
    ///
    /// * `defs` — used to rebuild unit baskets after the sell mix changes.
    /// * `state_id` — building’s state; `None` updates market totals only (100% access).
    /// * `old_inputs` / `old_outputs` — quantities to remove (previous `goods_io`).
    /// * `new_inputs` / `new_outputs` — quantities to add (updated `goods_io`).
    ///
    /// Also refreshes [`Self::units`] from the new market sell mix. Does **not**
    /// update [`Self::buildings`] — call [`Self::set_building_io`] for revenue rows.
    pub fn patch_building_io(
        &mut self,
        defs: &GameDefs,
        state_id: Option<u32>,
        old_inputs: &GoodsVec,
        old_outputs: &GoodsVec,
        new_inputs: &GoodsVec,
        new_outputs: &GoodsVec,
    ) {
        let access = state_id
            .and_then(|id| self.access_by_state.get(&id).copied())
            .unwrap_or(1.0);
        // Market totals are access-scaled (same as access_scaled_non_pop_orders).
        apply_io_delta(&mut self.frozen_buy, old_inputs, new_inputs, access);
        apply_io_delta(&mut self.frozen_sell, old_outputs, new_outputs, access);

        if let Some(id) = state_id {
            // Per-state shop stores unscaled orders; CoW-copy only this shop.
            let shop = self.ensure_shop(id, access);
            let shop = Arc::make_mut(shop);
            apply_io_delta(&mut shop.frozen_buy, old_inputs, new_inputs, 1.0);
            apply_io_delta(&mut shop.frozen_sell, old_outputs, new_outputs, 1.0);
        }

        self.rebuild_units(defs);
    }

    /// Insert or replace the per-building IO record used for revenues / GDP.
    ///
    /// * `building_id` — world building instance id.
    /// * `state_id` — optional state for local-price revenue.
    /// * `inputs` / `outputs` — full IO after the edit (not a delta).
    pub fn set_building_io(
        &mut self,
        building_id: u32,
        state_id: Option<u32>,
        inputs: GoodsVec,
        outputs: GoodsVec,
    ) {
        if let Some(slot) = self
            .buildings
            .iter_mut()
            .find(|row| row.building_id == building_id)
        {
            let row = Arc::make_mut(slot);
            row.state_id = state_id;
            row.inputs = inputs;
            row.outputs = outputs;
        } else {
            self.buildings.push(Arc::new(BuildingIo {
                building_id,
                state_id,
                inputs,
                outputs,
            }));
        }
    }

    /// Return a mutable Arc slot for `id`, creating an empty shop if missing.
    ///
    /// * `id` — state id.
    /// * `access` — market access used when creating a new shop.
    fn ensure_shop(&mut self, id: u32, access: f64) -> &mut Arc<StateShop> {
        if let Some(idx) = self.shops.iter().position(|shop| shop.id == id) {
            return &mut self.shops[idx];
        }
        let n = self.base_prices.len();
        self.shops.push(Arc::new(StateShop {
            id,
            access,
            mapi: effective_mapi(access),
            frozen_buy: GoodsVec::zeros(n),
            frozen_sell: GoodsVec::zeros(n),
            frozen_pop_buy: GoodsVec::zeros(n),
            wage_bins: Vec::new(),
        }));
        self.shops.last_mut().expect("just pushed")
    }

    /// Rebuild pop unit baskets from the current market sell mix.
    fn rebuild_units(&mut self, defs: &GameDefs) {
        let shares = NeedShares::from_sell(defs, &self.frozen_sell);
        self.units = UnitBaskets::from_shares(defs, &self.base_prices, &shares);
    }
}

/// Apply `new - old` into `target`, scaled by `scale` (1.0 for state shops, access for market).
fn apply_io_delta(target: &mut GoodsVec, old: &GoodsVec, new: &GoodsVec, scale: f64) {
    for (good, quantity) in old.iter_indexed() {
        target.add(good, -quantity * scale);
    }
    for (good, quantity) in new.iter_indexed() {
        target.add(good, quantity * scale);
    }
}

/// Non-pop orders reaching the single market after state access scaling.
///
/// * `world` — buildings, trade, and optional world-level frozen orders.
/// * `defs` — recipe IO for buildings.
/// * `access_by_state` — state id → access in `[0, 1]`.
///
/// Returns `(frozen_buy, frozen_sell)` at market level.
pub(crate) fn access_scaled_non_pop_orders(
    world: &World,
    defs: &GameDefs,
    access_by_state: &BTreeMap<u32, f64>,
) -> (GoodsVec, GoodsVec) {
    let n = defs.goods_order.len();
    // Start from world-level frozen extras (rare), then add trade + buildings.
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

/// Build one [`StateShop`] per relevant state id.
///
/// * `world` — source of buildings, trade, pops.
/// * `defs` — building IO recipes.
/// * `access_by_state` — known infra access (may be extended for orphan states).
/// * `n` — goods vector length.
/// * `base_prices` — for freezing non-wage pop buy.
/// * `units` — pop consumption baskets.
fn state_shops(
    world: &World,
    defs: &GameDefs,
    access_by_state: &BTreeMap<u32, f64>,
    n: usize,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
) -> Vec<Arc<StateShop>> {
    // Collect pops once — each state filters this list.
    let pops: Vec<WorldPop> = world.iter_pops().collect();

    // Union of state ids that need a shop: infra map, then pops/buildings/trade.
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
            // Trade orders for this state (unscaled; access applied at market fold).
            let mut frozen_buy = GoodsVec::zeros(n);
            let mut frozen_sell = GoodsVec::zeros(n);
            for trade in &world.state_trade {
                if trade.state == id {
                    trade.add_orders(&mut frozen_buy, &mut frozen_sell, 1.0);
                }
            }
            // Building IO for this state.
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
            // Pops: wage → bins; others → frozen buy at base prices.
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
            Arc::new(StateShop {
                id,
                access,
                mapi: effective_mapi(access),
                frozen_buy,
                frozen_sell,
                frozen_pop_buy,
                wage_bins: wage_bins_from_pops(wage_pops),
            })
        })
        .collect()
}

/// Stateless pops + access-scaled per-state frozen pop buy → market pop buy.
///
/// * `world` — for pops with `state == None`.
/// * `n` — goods length.
/// * `base_prices` / `units` — freeze non-wage stateless pops.
/// * `shops` — contribute each state’s `frozen_pop_buy * access`.
///
/// Returns `(stateless_wage_bins, market_frozen_pop_buy)`.
fn split_pop_buy(
    world: &World,
    n: usize,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    shops: &[Arc<StateShop>],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{WorldBuilding, WorldCountry, WorldState};
    use vic3_defs::{BuildingType, Good, ProductionMethod};

    fn tiny_defs() -> GameDefs {
        let mut defs = GameDefs {
            goods_order: vec!["grain".into(), "fabric".into()],
            goods: [
                (
                    "grain".into(),
                    Good {
                        name: "grain".into(),
                        base_price: 20.0,
                        traded_quantity: 0.0,
                        texture: None,
                    },
                ),
                (
                    "fabric".into(),
                    Good {
                        name: "fabric".into(),
                        base_price: 20.0,
                        traded_quantity: 0.0,
                        texture: None,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            price_range: 0.75,
            ..GameDefs::default()
        };
        defs.building_types.insert(
            "building_rye_farm".into(),
            BuildingType {
                name: "building_rye_farm".into(),
                group: None,
                city_type: None,
                production_method_groups: vec!["pmg_base".into()],
                required_construction: None,
            },
        );
        defs.production_methods.insert(
            "pm_grain".into(),
            ProductionMethod {
                name: "pm_grain".into(),
                outputs: vec![(defs.index_of("grain").unwrap(), 10.0)],
                ..ProductionMethod::default()
            },
        );
        defs.ensure_building_type("building_rye_farm");
        defs
    }

    fn tiny_world(defs: &GameDefs) -> World {
        assert!(
            defs.building_index_of("building_rye_farm").is_some(),
            "tiny_defs must register building_rye_farm"
        );
        let n = defs.goods_order.len();
        World {
            countries: vec![WorldCountry {
                id: 1,
                tag: "PRU".into(),
                ..WorldCountry::default()
            }],
            states: vec![WorldState {
                id: 10,
                country: Some(1),
                infrastructure: Some(10.0),
                infrastructure_usage: Some(0.0),
                ..WorldState::default()
            }],
            buildings: vec![WorldBuilding {
                id: 1,
                state: Some(10),
                building_type_id: defs
                    .building_index_of("building_rye_farm")
                    .expect("rye farm"),
                level: 1.0,
                staffing: 1.0,
                production_methods: vec!["pm_grain".into()],
                saved_inputs: Vec::new(),
                saved_outputs: Vec::new(),
            }],
            frozen_buy: GoodsVec::zeros(n),
            frozen_sell: GoodsVec::zeros(n),
            ..World::default()
        }
    }

    #[test]
    fn patch_building_io_matches_rebuild() {
        let defs = tiny_defs();
        let world = tiny_world(&defs);
        let mut cache = ShopCache::from_world(&world, &defs);

        let mut bumped = world.clone();
        bumped.buildings[0].add_extra_levels(1);
        let rebuilt = ShopCache::from_world(&bumped, &defs);

        let (old_i, old_o) = world.buildings[0].goods_io(&defs);
        let (new_i, new_o) = bumped.buildings[0].goods_io(&defs);
        cache.patch_building_io(&defs, Some(10), &old_i, &old_o, &new_i, &new_o);
        cache.set_building_io(1, Some(10), new_i.clone(), new_o.clone());

        let shop = cache.shops.iter().find(|s| s.id == 10).unwrap();
        let rebuilt_shop = rebuilt.shops.iter().find(|s| s.id == 10).unwrap();
        for (good, qty) in shop.frozen_sell.iter_indexed() {
            assert!(
                (qty - rebuilt_shop.frozen_sell[good]).abs() < 1e-9,
                "sell mismatch good {good:?}"
            );
        }
        for (good, qty) in cache.frozen_sell.iter_indexed() {
            assert!(
                (qty - rebuilt.frozen_sell[good]).abs() < 1e-9,
                "market sell mismatch good {good:?}"
            );
        }
        assert_eq!(
            cache.buildings[0].outputs[defs.index_of("grain").unwrap()],
            new_o[defs.index_of("grain").unwrap()]
        );
    }
}
