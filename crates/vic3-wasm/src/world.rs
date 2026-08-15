//! Local `vic3_load::Save` → `vic3_prices::World` adapter.
//!
//! `World::from_save` may land in `vic3-prices` (P5a). Until then this crate
//! keeps a small projection so wasm does not wait on that API.

use std::collections::BTreeMap;

use vic3_load::{Pop, Save};
use vic3_prices::{World, WorldBuilding, WorldPop};

/// Project save IR into the price-solver world.
///
/// Building production methods are not on the IR yet, so PM id is empty and
/// those buildings contribute no goods IO. Trade-route volumes are treated as
/// frozen sell (employment / wages stay frozen at save values; wages are 0).
pub(crate) fn world_from_save(save: &Save) -> World {
    World {
        pops: save
            .pops
            .iter_present()
            .map(|(_, pop)| world_pop(pop))
            .collect(),
        buildings: save
            .building_manager
            .iter_present()
            .map(|(_, building)| WorldBuilding {
                building: building.building.clone(),
                level: f64::from(building.level),
                staffing: building.staffing,
                production_method: String::new(),
            })
            .collect(),
        frozen_buy: BTreeMap::new(),
        frozen_sell: trade_sell(save),
    }
}

fn world_pop(pop: &Pop) -> WorldPop {
    let wealth = pop.wealth.unwrap_or(1).clamp(1, 99) as u8;
    WorldPop {
        size: pop.size.unwrap_or(0.0),
        wealth,
        wages: 0.0,
        culture: pop.culture.clone(),
    }
}

fn trade_sell(save: &Save) -> BTreeMap<String, f64> {
    let mut sell = BTreeMap::new();
    for (_, route) in save.trade_route_manager.iter_present() {
        let Some(good) = route.goods.as_ref() else {
            continue;
        };
        let Some(volume) = route.volume else {
            continue;
        };
        *sell.entry(good.clone()).or_default() += volume;
    }
    sell
}
