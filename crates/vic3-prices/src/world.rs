//! Synthetic (and later IR-backed) market: pops, buildings, frozen orders.

use std::collections::BTreeMap;

use vic3_defs::GameDefs;

/// Pop size unit for buy packages (Vic3: package values are per 10k working pops).
pub const POP_SCALE: f64 = 10_000.0;

/// Market snapshot owned by this crate. Can be filled from `vic3-load` IR later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct World {
    pub pops: Vec<WorldPop>,
    pub buildings: Vec<WorldBuilding>,
    /// Government / trade / construction buy orders, held fixed during the solve.
    pub frozen_buy: BTreeMap<String, f64>,
    /// Trade (and any other non-building) sell orders, held fixed during the solve.
    pub frozen_sell: BTreeMap<String, f64>,
}

/// A pop whose consumption sits in the price loop.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldPop {
    pub size: f64,
    /// Saved wealth 1–99. Used as the Laspeyres reference basket.
    pub wealth: u8,
    /// Frozen wage bill. When `≤ 0`, wealth stays at [`Self::wealth`].
    pub wages: f64,
    pub culture: Option<String>,
}

/// A building whose goods IO is reconstructed from defs PMs and then frozen
/// (employment = [`Self::staffing`] does not change in [`crate::what_if`]).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldBuilding {
    pub building: String,
    pub level: f64,
    /// Employment / throughput fraction. Frozen except that what-if does not touch it.
    pub staffing: f64,
    pub production_method: String,
}

/// Non-pop buy and sell orders: frozen maps plus building PM inputs/outputs.
///
/// Building volumes use `level * staffing` and current PM recipes. They are
/// held fixed for a given [`World`]; [`crate::what_if`] clones the world and
/// bumps `level` only.
pub fn reconstruct_non_pop_orders(
    world: &World,
    defs: &GameDefs,
) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
    let mut buy = world.frozen_buy.clone();
    let mut sell = world.frozen_sell.clone();
    for building in &world.buildings {
        let Some(pm) = defs.production_methods.get(&building.production_method) else {
            continue;
        };
        let scale = building.level * building.staffing;
        if scale == 0.0 {
            continue;
        }
        for (good, qty) in &pm.inputs {
            *buy.entry(good.clone()).or_default() += *qty * scale;
        }
        for (good, qty) in &pm.outputs {
            *sell.entry(good.clone()).or_default() += *qty * scale;
        }
    }
    (buy, sell)
}

impl World {
    /// Clone this world and add `extra_levels` to every building of `building` type.
    ///
    /// Staffing (employment) is left unchanged.
    pub fn with_extra_levels(&self, building: &str, extra_levels: u32) -> Self {
        let mut next = self.clone();
        let extra = f64::from(extra_levels);
        for b in &mut next.buildings {
            if b.building == building {
                b.level += extra;
            }
        }
        next
    }
}
