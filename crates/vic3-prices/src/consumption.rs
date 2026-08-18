//! Pop consumption from buy packages, substitution, and relaxed wealth.

use vic3_defs::{substitution_shares, GameDefs, GoodIdx, GoodsVec, NeedIdx, NeedsVec, PopNeed};

use crate::world::{WorldPop, POP_SCALE};

/// One need's goods after package interpolation and substitution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NeedBasket {
    pub need_idx: NeedIdx,
    pub package_value: f64,
    pub goods: Vec<(GoodIdx, f64)>,
}

/// Pop buy orders at `prices` (good id → quantity).
///
/// `prices` are the prices the household pays — market prices for a single
/// world shop, or a state's MAPI-blended local prices in the residual.
///
/// Wealth 1–99 is **relaxed continuous** (Laspeyres real-income scaling when
/// wages are set) then **interpolated** between neighboring buy packages — not
/// an ILP over the discrete ladder. Substitution uses
/// [`vic3_defs::substitution_shares`] on each need's **world** sell-order shares.
pub fn consumption(
    pops: &[WorldPop],
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
) -> GoodsVec {
    let mut buy = GoodsVec::zeros(defs.goods_order.len());
    for pop in pops {
        add_pop_consumption(&mut buy, pop, prices, base_prices, defs, sell_orders);
    }
    buy
}

/// Add one pop's buy orders into `buy` (used by the residual wage-pop path).
pub(crate) fn add_pop_consumption(
    buy: &mut GoodsVec,
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
) {
    add_pop_consumption_scaled(buy, pop, prices, base_prices, defs, sell_orders, 1.0);
}

/// Add one pop's buy orders multiplied by its market-access contribution.
pub(crate) fn add_pop_consumption_scaled(
    buy: &mut GoodsVec,
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
    order_scale: f64,
) {
    if pop.size <= 0.0 || order_scale <= 0.0 {
        return;
    }
    for basket in pop_need_baskets(pop, prices, base_prices, defs, sell_orders) {
        for (good, qty) in basket.goods {
            buy.add(good, qty * order_scale);
        }
    }
}

/// Per-need goods basket for one pop at the given prices / sell orders.
pub(crate) fn pop_need_baskets(
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
) -> Vec<NeedBasket> {
    if pop.size <= 0.0 {
        return Vec::new();
    }
    let wealth = continuous_wealth(pop, prices, base_prices, defs, sell_orders);
    let needs = package_needs(wealth, defs);
    let scale = pop.size / POP_SCALE;
    let mut baskets = Vec::new();
    for (need_idx, package_value) in needs.iter_indexed() {
        if package_value == 0.0 {
            continue;
        }
        let Some(need) = defs.need_by_index(need_idx) else {
            continue;
        };
        let Some(qty) = need_quantity(need, package_value, scale, base_prices) else {
            continue;
        };
        if qty == 0.0 {
            continue;
        }
        let shares = substitution_shares(need, sell_orders);
        let mut goods = Vec::new();
        apply_need_shares_into(&mut goods, need, qty, &shares);
        if !goods.is_empty() {
            baskets.push(NeedBasket {
                need_idx,
                package_value,
                goods,
            });
        }
    }
    baskets
}

fn apply_need_shares_into(
    goods: &mut Vec<(GoodIdx, f64)>,
    need: &PopNeed,
    qty: f64,
    shares: &[(GoodIdx, f64)],
) {
    let share_sum: f64 = shares.iter().map(|(_, share)| *share).sum();
    if share_sum <= 0.0 {
        if let Some(default) = need
            .default_good
            .or_else(|| need.entries.first().map(|e| e.good))
        {
            goods.push((default, qty));
        }
        return;
    }
    for (good, share) in shares {
        if *share > 0.0 {
            goods.push((*good, qty * share));
        }
    }
}

/// Continuous wealth in `[1, 99]`, then used to interpolate buy packages.
///
/// When `pop.wages ≤ 0`, wealth is frozen at the saved integer. Otherwise
/// `wealth = saved * base_col / col` with a Laspeyres basket at saved wealth.
fn continuous_wealth(
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
) -> f64 {
    let saved = f64::from(pop.wealth).clamp(1.0, 99.0);
    if pop.wages <= 0.0 {
        return saved;
    }
    let basket = basket_quantities(pop.size, saved, base_prices, defs, sell_orders);
    let mut col = 0.0;
    let mut base_col = 0.0;
    for (good, qty) in basket.iter_indexed() {
        let base = base_prices[good];
        let p = prices[good];
        col += qty * p;
        base_col += qty * base;
    }
    if col <= 0.0 || base_col <= 0.0 {
        return saved;
    }
    (saved * base_col / col).clamp(1.0, 99.0)
}

fn basket_quantities(
    size: f64,
    wealth: f64,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
) -> GoodsVec {
    let mut buy = GoodsVec::zeros(defs.goods_order.len());
    let needs = package_needs(wealth, defs);
    let scale = size / POP_SCALE;
    for (need_idx, package_value) in needs.iter_indexed() {
        if package_value == 0.0 {
            continue;
        }
        let Some(need) = defs.need_by_index(need_idx) else {
            continue;
        };
        let Some(qty) = need_quantity(need, package_value, scale, base_prices) else {
            continue;
        };
        let shares = substitution_shares(need, sell_orders);
        apply_need_shares(&mut buy, need, qty, &shares);
    }
    buy
}

fn apply_need_shares(
    buy: &mut GoodsVec,
    need: &PopNeed,
    qty: f64,
    shares: &[(vic3_defs::GoodIdx, f64)],
) {
    let share_sum: f64 = shares.iter().map(|(_, share)| *share).sum();
    if share_sum <= 0.0 {
        if let Some(default) = need
            .default_good
            .or_else(|| need.entries.first().map(|e| e.good))
        {
            buy.add(default, qty);
        }
        return;
    }
    for (good, share) in shares {
        if *share > 0.0 {
            buy.add(*good, qty * share);
        }
    }
}

/// Interpolated need package values at continuous `wealth` (no String clones).
fn package_needs(wealth: f64, defs: &GameDefs) -> NeedsVec {
    let n = defs.needs_order.len();
    if defs.package_ladder.is_empty() || n == 0 {
        return NeedsVec::zeros(n);
    }
    let max_w = defs.package_ladder.len() as f64;
    let w = wealth.clamp(1.0, max_w);
    let lo = w.floor();
    let hi = w.ceil();
    let i_lo = (lo as usize).saturating_sub(1);
    let i_hi = (hi as usize).saturating_sub(1);
    if i_lo == i_hi {
        return defs.package_ladder[i_lo].clone();
    }
    let t = w - lo;
    NeedsVec::lerp(&defs.package_ladder[i_lo], &defs.package_ladder[i_hi], t)
}

fn need_quantity(
    need: &PopNeed,
    package_value: f64,
    scale: f64,
    base_prices: &GoodsVec,
) -> Option<f64> {
    if package_value == 0.0 || scale == 0.0 {
        return Some(0.0);
    }
    let default_id = need
        .default_good
        .or_else(|| need.entries.first().map(|e| e.good))?;
    let base = base_prices[default_id];
    if base <= 0.0 {
        return None;
    }
    Some(package_value / base * scale)
}
