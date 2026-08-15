//! Pop consumption from buy packages, substitution, and relaxed wealth.

use std::collections::{BTreeMap, BTreeSet};

use vic3_defs::{substitution_shares, GameDefs, PopNeed};

use crate::world::{WorldPop, POP_SCALE};

/// Pop buy orders at `prices` (good id → quantity).
///
/// Wealth 1–99 is **relaxed continuous** (Laspeyres real-income scaling when
/// wages are set) then **interpolated** between neighboring buy packages — not
/// an ILP over the discrete ladder. Substitution uses
/// [`vic3_defs::substitution_shares`] on each need's sell-order shares.
pub fn consumption(
    pops: &[WorldPop],
    prices: &BTreeMap<String, f64>,
    defs: &GameDefs,
    sell_orders: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    let mut buy = BTreeMap::new();
    for pop in pops {
        add_pop_consumption(&mut buy, pop, prices, defs, sell_orders);
    }
    buy
}

fn add_pop_consumption(
    buy: &mut BTreeMap<String, f64>,
    pop: &WorldPop,
    prices: &BTreeMap<String, f64>,
    defs: &GameDefs,
    sell_orders: &BTreeMap<String, f64>,
) {
    if pop.size <= 0.0 {
        return;
    }
    let wealth = continuous_wealth(pop, prices, defs, sell_orders);
    let needs = package_needs(wealth, defs);
    let scale = pop.size / POP_SCALE;
    for (need_id, package_value) in needs {
        let Some(need) = defs.pop_needs.get(&need_id) else {
            continue;
        };
        let Some(qty) = need_quantity(need, package_value, scale, defs) else {
            continue;
        };
        if qty == 0.0 {
            continue;
        }
        let shares = substitution_shares(need, &need_sell_shares(need, sell_orders));
        apply_need_shares(buy, need, qty, &shares);
    }
}

/// Continuous wealth in `[1, 99]`, then used to interpolate buy packages.
///
/// When `pop.wages ≤ 0`, wealth is frozen at the saved integer. Otherwise
/// `wealth = saved * base_col / col` with a Laspeyres basket at saved wealth.
fn continuous_wealth(
    pop: &WorldPop,
    prices: &BTreeMap<String, f64>,
    defs: &GameDefs,
    sell_orders: &BTreeMap<String, f64>,
) -> f64 {
    let saved = f64::from(pop.wealth).clamp(1.0, 99.0);
    if pop.wages <= 0.0 {
        return saved;
    }
    let basket = basket_quantities(pop.size, saved, defs, sell_orders);
    let mut col = 0.0;
    let mut base_col = 0.0;
    for (good, qty) in basket {
        let base = defs.base_price(&good).unwrap_or(0.0);
        let p = prices.get(&good).copied().unwrap_or(base);
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
    defs: &GameDefs,
    sell_orders: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    let mut buy = BTreeMap::new();
    let needs = package_needs(wealth, defs);
    let scale = size / POP_SCALE;
    for (need_id, package_value) in needs {
        let Some(need) = defs.pop_needs.get(&need_id) else {
            continue;
        };
        let Some(qty) = need_quantity(need, package_value, scale, defs) else {
            continue;
        };
        let shares = substitution_shares(need, &need_sell_shares(need, sell_orders));
        apply_need_shares(&mut buy, need, qty, &shares);
    }
    buy
}

fn apply_need_shares(
    buy: &mut BTreeMap<String, f64>,
    need: &PopNeed,
    qty: f64,
    shares: &BTreeMap<String, f64>,
) {
    let share_sum: f64 = shares.values().sum();
    if share_sum <= 0.0 {
        if let Some(default) = need
            .default_good
            .clone()
            .or_else(|| need.entries.first().map(|e| e.good.clone()))
        {
            *buy.entry(default).or_default() += qty;
        }
        return;
    }
    for (good, share) in shares {
        if *share > 0.0 {
            *buy.entry(good.clone()).or_default() += qty * share;
        }
    }
}

fn package_needs(wealth: f64, defs: &GameDefs) -> BTreeMap<String, f64> {
    if defs.buy_packages.is_empty() {
        return BTreeMap::new();
    }
    let keys: Vec<u8> = defs.buy_packages.keys().copied().collect();
    let min_w = f64::from(keys[0]);
    let max_w = f64::from(*keys.last().expect("non-empty keys"));
    let w = wealth.clamp(min_w, max_w);

    let mut lo = keys[0];
    let mut hi = keys[0];
    for &k in &keys {
        if f64::from(k) <= w {
            lo = k;
        }
        if f64::from(k) >= w {
            hi = k;
            break;
        }
        hi = k;
    }

    let p_lo = &defs.buy_packages[&lo].needs;
    if lo == hi {
        return p_lo.clone();
    }
    let span = f64::from(hi) - f64::from(lo);
    if span <= 0.0 {
        return p_lo.clone();
    }
    let t = (w - f64::from(lo)) / span;
    let p_hi = &defs.buy_packages[&hi].needs;
    let mut ids: BTreeSet<&String> = BTreeSet::new();
    ids.extend(p_lo.keys());
    ids.extend(p_hi.keys());
    ids.into_iter()
        .map(|need| {
            let a = p_lo.get(need).copied().unwrap_or(0.0);
            let b = p_hi.get(need).copied().unwrap_or(0.0);
            (need.clone(), a * (1.0 - t) + b * t)
        })
        .collect()
}

fn need_quantity(need: &PopNeed, package_value: f64, scale: f64, defs: &GameDefs) -> Option<f64> {
    if package_value == 0.0 || scale == 0.0 {
        return Some(0.0);
    }
    let default_id = need
        .default_good
        .as_deref()
        .or_else(|| need.entries.first().map(|e| e.good.as_str()))?;
    let base = defs.base_price(default_id)?;
    if base <= 0.0 {
        return None;
    }
    Some(package_value / base * scale)
}

fn need_sell_shares(need: &PopNeed, sell_orders: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    let total: f64 = need
        .entries
        .iter()
        .map(|e| sell_orders.get(&e.good).copied().unwrap_or(0.0).max(0.0))
        .sum();
    need.entries
        .iter()
        .map(|e| {
            let s = sell_orders.get(&e.good).copied().unwrap_or(0.0).max(0.0);
            let share = if total > 0.0 { s / total } else { 0.0 };
            (e.good.clone(), share)
        })
        .collect()
}
