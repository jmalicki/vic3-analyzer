//! Pop consumption from buy packages, substitution, and relaxed wealth.
//!
//! This is the **in-loop** demand side of the equilibrium: buildings and trade
//! stay frozen; pops adjust. Two caches share substitution shares and the
//! 1..=99 wealth ladder:
//! - [`UnitBaskets`] — dense goods totals for the residual (scale/lerp).
//! - [`UnitNeedBaskets`] — per-need goods for the population tab.
//!
//! The price loop does not need per-need rows. The webapp JSON does, so
//! [`pop_compact_needs`] / [`UnitNeedBaskets::compact_for`] still emit a scaled
//! copy per collapsed group rather than omitting baskets. Rebuilding shares
//! per group was the old cost; after the cache, `finished` is grouping and
//! those copies.

use vic3_defs::{substitution_shares, GameDefs, GoodIdx, GoodsVec, NeedIdx, NeedsVec, PopNeed};

use crate::result::CompactNeed;
use crate::world::{WorldPop, POP_SCALE};

/// One need's goods after package interpolation and substitution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NeedBasket {
    pub need_idx: NeedIdx,
    pub package_value: f64,
    pub goods: Vec<(GoodIdx, f64)>,
}

/// Substitution shares for every need, computed once from frozen world sell.
#[derive(Debug, Clone)]
pub(crate) struct NeedShares {
    by_need: Vec<Vec<(GoodIdx, f64)>>,
}

impl NeedShares {
    pub(crate) fn from_sell(defs: &GameDefs, sell_orders: &GoodsVec) -> Self {
        let by_need = (0..defs.needs_order.len())
            .map(|i| {
                defs.need_by_index(NeedIdx::from_usize(i))
                    .map(|need| substitution_shares(need, sell_orders))
                    .unwrap_or_default()
            })
            .collect();
        Self { by_need }
    }

    fn get(&self, idx: NeedIdx) -> &[(GoodIdx, f64)] {
        self.by_need
            .get(idx.as_usize())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Dense goods baskets for a pop of size [`POP_SCALE`] at each integer wealth.
///
/// Quantity is linear in pop size; Laspeyres wealth does not depend on size.
/// Wage pops with the same saved wealth therefore share one continuous wealth
/// at a given price vector — see [`WealthBin`].
#[derive(Debug, Clone)]
pub(crate) struct UnitBaskets {
    /// Index 0 unused; `1..=max_wealth` are integer-wealth unit vectors.
    by_wealth: Vec<GoodsVec>,
}

/// Per-need goods for a pop of size [`POP_SCALE`] at each integer wealth.
///
/// [`pop_compact_needs`] scales and, for continuous Laspeyres wealth, lerps these
/// unit rows so UI need baskets match the residual's [`UnitBaskets`]. Building
/// the 99-row table is cheap next to walking every pop group; each call still
/// allocates the scaled `Vec` because that is the JSON payload.
#[derive(Debug, Clone)]
pub(crate) struct UnitNeedBaskets {
    /// Index 0 unused; `1..=max_wealth` are integer-wealth need lists.
    by_wealth: Vec<Vec<NeedBasket>>,
}

impl UnitBaskets {
    pub(crate) fn from_shares(
        defs: &GameDefs,
        base_prices: &GoodsVec,
        shares: &NeedShares,
    ) -> Self {
        let max_wealth = defs.package_ladder.len().max(1);
        let mut by_wealth = Vec::with_capacity(max_wealth + 1);
        by_wealth.push(GoodsVec::zeros(defs.goods_order.len()));
        for wealth in 1..=max_wealth {
            by_wealth.push(unit_basket_at(wealth as f64, defs, base_prices, shares));
        }
        Self { by_wealth }
    }

    fn max_wealth(&self) -> f64 {
        (self.by_wealth.len().saturating_sub(1) as f64).max(1.0)
    }

    fn at(&self, wealth: u8) -> &GoodsVec {
        let max = self.by_wealth.len().saturating_sub(1).max(1);
        &self.by_wealth[(wealth as usize).clamp(1, max)]
    }

    fn continuous_wealth(&self, saved: u8, prices: &GoodsVec, base_prices: &GoodsVec) -> f64 {
        let saved_f = f64::from(saved).clamp(1.0, self.max_wealth());
        let unit = self.at(saved);
        let mut col = 0.0;
        let mut base_col = 0.0;
        for (good, qty) in unit.iter_indexed() {
            col += qty * prices[good];
            base_col += qty * base_prices[good];
        }
        if col <= 0.0 || base_col <= 0.0 {
            return saved_f;
        }
        (saved_f * base_col / col).clamp(1.0, self.max_wealth())
    }

    fn add_at_wealth(&self, buy: &mut GoodsVec, wealth: f64, scale: f64) {
        if scale == 0.0 {
            return;
        }
        let w = wealth.clamp(1.0, self.max_wealth());
        let lo = w.floor();
        let hi = w.ceil();
        let i_lo = lo as u8;
        let i_hi = hi as u8;
        if i_lo == i_hi {
            buy.add_scaled(self.at(i_lo), scale);
            return;
        }
        let t = w - lo;
        buy.add_scaled(self.at(i_lo), scale * (1.0 - t));
        buy.add_scaled(self.at(i_hi), scale * t);
    }
}

impl UnitNeedBaskets {
    pub(crate) fn from_shares(
        defs: &GameDefs,
        base_prices: &GoodsVec,
        shares: &NeedShares,
    ) -> Self {
        let max_wealth = defs.package_ladder.len().max(1);
        let mut by_wealth = Vec::with_capacity(max_wealth + 1);
        by_wealth.push(Vec::new());
        for wealth in 1..=max_wealth {
            by_wealth.push(need_baskets_at(
                wealth as f64,
                1.0,
                defs,
                base_prices,
                shares,
            ));
        }
        Self { by_wealth }
    }

    fn max_wealth(&self) -> f64 {
        (self.by_wealth.len().saturating_sub(1) as f64).max(1.0)
    }

    fn at(&self, wealth: u8) -> &[NeedBasket] {
        let max = self.by_wealth.len().saturating_sub(1).max(1);
        &self.by_wealth[(wealth as usize).clamp(1, max)]
    }

    #[cfg(test)]
    fn scaled_for(&self, wealth: f64, scale: f64) -> Vec<NeedBasket> {
        if scale == 0.0 {
            return Vec::new();
        }
        let w = wealth.clamp(1.0, self.max_wealth());
        let lo = w.floor();
        let hi = w.ceil();
        let i_lo = lo as u8;
        let i_hi = hi as u8;
        if i_lo == i_hi {
            return scale_need_baskets(self.at(i_lo), scale);
        }
        lerp_need_baskets(self.at(i_lo), self.at(i_hi), w - lo, scale)
    }

    /// Scale/lerp unit need rows into UI compact needs, including spend.
    ///
    /// One allocation per need/goods vec (the JSON payload) instead of
    /// [`NeedBasket`] then a second copy with `qty * price`.
    pub(crate) fn compact_for(
        &self,
        wealth: f64,
        scale: f64,
        prices: &GoodsVec,
    ) -> Vec<CompactNeed> {
        if scale == 0.0 {
            return Vec::new();
        }
        let w = wealth.clamp(1.0, self.max_wealth());
        let lo = w.floor();
        let hi = w.ceil();
        let i_lo = lo as u8;
        let i_hi = hi as u8;
        if i_lo == i_hi {
            return compact_need_baskets(self.at(i_lo), scale, prices);
        }
        compact_lerp_need_baskets(self.at(i_lo), self.at(i_hi), w - lo, scale, prices)
    }
}

/// Wage pops that share a saved integer wealth, so they share a unit basket.
///
/// Storage is a single `(wealth, total_size)` row; the residual adds
/// `unit[wealth] * size / POP_SCALE` (after Laspeyres) without walking pops.
#[derive(Debug, Clone)]
pub(crate) struct WealthBin {
    pub wealth: u8,
    pub size: f64,
}

impl WealthBin {
    fn add_into(
        &self,
        buy: &mut GoodsVec,
        prices: &GoodsVec,
        base_prices: &GoodsVec,
        units: &UnitBaskets,
        order_scale: f64,
    ) {
        let scale = self.size / POP_SCALE * order_scale;
        if scale <= 0.0 {
            return;
        }
        let wealth = units.continuous_wealth(self.wealth, prices, base_prices);
        units.add_at_wealth(buy, wealth, scale);
    }
}

/// Collapse wage pops into bins keyed by saved integer wealth.
pub(crate) fn wage_bins_from_pops<'a, I>(pops: I) -> Vec<WealthBin>
where
    I: IntoIterator<Item = &'a WorldPop>,
{
    let mut sizes = [0.0; 100];
    for pop in pops {
        if pop.wages > 0.0 && pop.size > 0.0 {
            sizes[usize::from(pop.wealth.clamp(1, 99))] += pop.size;
        }
    }
    sizes
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, size)| **size > 0.0)
        .map(|(wealth, size)| WealthBin {
            wealth: wealth as u8,
            size: *size,
        })
        .collect()
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
///
/// # Arguments
///
/// * `pops` — consumption views ([`WorldPop`]); size scaled by [`POP_SCALE`].
/// * `prices` / `base_prices` — paid prices and bases (same goods order as `defs`).
/// * `sell_orders` — world sell vector that drives substitution shares (frozen
///   building/trade sell in the residual, not local attributed sell).
pub fn consumption(
    pops: &[WorldPop],
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    defs: &GameDefs,
    sell_orders: &GoodsVec,
) -> GoodsVec {
    let shares = NeedShares::from_sell(defs, sell_orders);
    let units = UnitBaskets::from_shares(defs, base_prices, &shares);
    let mut buy = GoodsVec::zeros(defs.goods_order.len());
    for pop in pops {
        add_pop_from_units(&mut buy, pop, prices, base_prices, &units, 1.0);
    }
    buy
}

pub(crate) fn add_pop_from_units(
    buy: &mut GoodsVec,
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    order_scale: f64,
) {
    let scale = pop.size / POP_SCALE * order_scale;
    if scale <= 0.0 {
        return;
    }
    let wealth = if pop.wages <= 0.0 {
        f64::from(pop.wealth).clamp(1.0, units.max_wealth())
    } else {
        units.continuous_wealth(pop.wealth, prices, base_prices)
    };
    units.add_at_wealth(buy, wealth, scale);
}

/// Add wage-bin buy (Laspeyres wealth) into `buy`.
pub(crate) fn add_wage_bins(
    buy: &mut GoodsVec,
    bins: &[WealthBin],
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    order_scale: f64,
) {
    for bin in bins {
        bin.add_into(buy, prices, base_prices, units, order_scale);
    }
}

/// Per-need goods basket for one pop at the given prices / cached unit needs.
///
/// Output matches rebuilding from [`NeedShares`] + package lerp. Used by unit
/// tests to check the wealth ladder against a direct rebuild.
#[cfg(test)]
pub(crate) fn pop_need_baskets(
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    need_units: &UnitNeedBaskets,
) -> Vec<NeedBasket> {
    if pop.size <= 0.0 {
        return Vec::new();
    }
    need_units.scaled_for(
        pop_wealth(pop, prices, base_prices, units),
        pop.size / POP_SCALE,
    )
}

/// Same quantities as [`pop_need_baskets`], with spend filled for compact UI rows.
pub(crate) fn pop_compact_needs(
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
    need_units: &UnitNeedBaskets,
) -> Vec<CompactNeed> {
    if pop.size <= 0.0 {
        return Vec::new();
    }
    need_units.compact_for(
        pop_wealth(pop, prices, base_prices, units),
        pop.size / POP_SCALE,
        prices,
    )
}

fn pop_wealth(
    pop: &WorldPop,
    prices: &GoodsVec,
    base_prices: &GoodsVec,
    units: &UnitBaskets,
) -> f64 {
    if pop.wages <= 0.0 {
        f64::from(pop.wealth).clamp(1.0, units.max_wealth())
    } else {
        units.continuous_wealth(pop.wealth, prices, base_prices)
    }
}

fn need_baskets_at(
    wealth: f64,
    scale: f64,
    defs: &GameDefs,
    base_prices: &GoodsVec,
    shares: &NeedShares,
) -> Vec<NeedBasket> {
    let needs = package_needs(wealth, defs);
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
        let mut goods = Vec::new();
        apply_need_shares_into(&mut goods, need, qty, shares.get(need_idx));
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

#[cfg(test)]
fn scale_need_baskets(unit: &[NeedBasket], scale: f64) -> Vec<NeedBasket> {
    if scale == 1.0 {
        return unit.to_vec();
    }
    unit.iter()
        .map(|basket| scale_need_basket(basket, scale))
        .collect()
}

#[cfg(test)]
fn scale_need_basket(basket: &NeedBasket, scale: f64) -> NeedBasket {
    NeedBasket {
        need_idx: basket.need_idx,
        package_value: basket.package_value,
        goods: basket
            .goods
            .iter()
            .map(|(good, qty)| (*good, qty * scale))
            .collect(),
    }
}

#[cfg(test)]
fn lerp_need_baskets(lo: &[NeedBasket], hi: &[NeedBasket], t: f64, scale: f64) -> Vec<NeedBasket> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < lo.len() || j < hi.len() {
        match (lo.get(i), hi.get(j)) {
            (Some(a), Some(b)) if a.need_idx == b.need_idx => {
                out.push(lerp_need_basket(a, b, t, scale));
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a.need_idx < b.need_idx => {
                out.push(scale_need_basket(a, scale * (1.0 - t)));
                i += 1;
            }
            (Some(a), None) => {
                out.push(scale_need_basket(a, scale * (1.0 - t)));
                i += 1;
            }
            (Some(_), Some(b)) => {
                out.push(scale_need_basket(b, scale * t));
                j += 1;
            }
            (None, Some(b)) => {
                out.push(scale_need_basket(b, scale * t));
                j += 1;
            }
            (None, None) => break,
        }
    }
    out
}

#[cfg(test)]
fn lerp_need_basket(lo: &NeedBasket, hi: &NeedBasket, t: f64, scale: f64) -> NeedBasket {
    NeedBasket {
        need_idx: lo.need_idx,
        package_value: (1.0 - t) * lo.package_value + t * hi.package_value,
        goods: lerp_need_goods(&lo.goods, &hi.goods, t, scale),
    }
}

fn compact_need_baskets(unit: &[NeedBasket], scale: f64, prices: &GoodsVec) -> Vec<CompactNeed> {
    unit.iter()
        .map(|basket| compact_need_basket(basket, scale, prices))
        .collect()
}

fn compact_need_basket(basket: &NeedBasket, scale: f64, prices: &GoodsVec) -> CompactNeed {
    CompactNeed {
        need_idx: basket.need_idx,
        package_value: basket.package_value,
        goods: basket
            .goods
            .iter()
            .map(|(good, qty)| {
                let quantity = qty * scale;
                (*good, quantity, quantity * prices[*good])
            })
            .collect(),
    }
}

fn compact_lerp_need_baskets(
    lo: &[NeedBasket],
    hi: &[NeedBasket],
    t: f64,
    scale: f64,
    prices: &GoodsVec,
) -> Vec<CompactNeed> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < lo.len() || j < hi.len() {
        match (lo.get(i), hi.get(j)) {
            (Some(a), Some(b)) if a.need_idx == b.need_idx => {
                out.push(compact_lerp_need_basket(a, b, t, scale, prices));
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a.need_idx < b.need_idx => {
                out.push(compact_need_basket(a, scale * (1.0 - t), prices));
                i += 1;
            }
            (Some(a), None) => {
                out.push(compact_need_basket(a, scale * (1.0 - t), prices));
                i += 1;
            }
            (Some(_), Some(b)) => {
                out.push(compact_need_basket(b, scale * t, prices));
                j += 1;
            }
            (None, Some(b)) => {
                out.push(compact_need_basket(b, scale * t, prices));
                j += 1;
            }
            (None, None) => break,
        }
    }
    out
}

fn compact_lerp_need_basket(
    lo: &NeedBasket,
    hi: &NeedBasket,
    t: f64,
    scale: f64,
    prices: &GoodsVec,
) -> CompactNeed {
    CompactNeed {
        need_idx: lo.need_idx,
        package_value: (1.0 - t) * lo.package_value + t * hi.package_value,
        goods: compact_lerp_need_goods(&lo.goods, &hi.goods, t, scale, prices),
    }
}

fn compact_lerp_need_goods(
    lo: &[(GoodIdx, f64)],
    hi: &[(GoodIdx, f64)],
    t: f64,
    scale: f64,
    prices: &GoodsVec,
) -> Vec<(GoodIdx, f64, f64)> {
    if lo.len() == hi.len() && lo.iter().zip(hi).all(|(a, b)| a.0 == b.0) {
        return lo
            .iter()
            .zip(hi)
            .map(|((good, a), (_, b))| {
                let quantity = scale * ((1.0 - t) * a + t * b);
                (*good, quantity, quantity * prices[*good])
            })
            .collect();
    }
    lerp_need_goods(lo, hi, t, scale)
        .into_iter()
        .map(|(good, quantity)| (good, quantity, quantity * prices[good]))
        .collect()
}

fn lerp_need_goods(
    lo: &[(GoodIdx, f64)],
    hi: &[(GoodIdx, f64)],
    t: f64,
    scale: f64,
) -> Vec<(GoodIdx, f64)> {
    if lo.len() == hi.len() && lo.iter().zip(hi).all(|(a, b)| a.0 == b.0) {
        return lo
            .iter()
            .zip(hi)
            .map(|((good, a), (_, b))| (*good, scale * ((1.0 - t) * a + t * b)))
            .collect();
    }
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < lo.len() || j < hi.len() {
        match (lo.get(i), hi.get(j)) {
            (Some(&(g, a)), Some(&(h, b))) if g == h => {
                out.push((g, scale * ((1.0 - t) * a + t * b)));
                i += 1;
                j += 1;
            }
            (Some(&(g, a)), Some(&(h, _))) if g < h => {
                out.push((g, scale * (1.0 - t) * a));
                i += 1;
            }
            (Some(&(g, a)), None) => {
                out.push((g, scale * (1.0 - t) * a));
                i += 1;
            }
            (Some(_), Some(&(h, b))) => {
                out.push((h, scale * t * b));
                j += 1;
            }
            (None, Some(&(h, b))) => {
                out.push((h, scale * t * b));
                j += 1;
            }
            (None, None) => break,
        }
    }
    out
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

fn unit_basket_at(
    wealth: f64,
    defs: &GameDefs,
    base_prices: &GoodsVec,
    shares: &NeedShares,
) -> GoodsVec {
    let mut buy = GoodsVec::zeros(defs.goods_order.len());
    let needs = package_needs(wealth, defs);
    for (need_idx, package_value) in needs.iter_indexed() {
        if package_value == 0.0 {
            continue;
        }
        let Some(need) = defs.need_by_index(need_idx) else {
            continue;
        };
        let Some(qty) = need_quantity(need, package_value, 1.0, base_prices) else {
            continue;
        };
        apply_need_shares(&mut buy, need, qty, shares.get(need_idx));
    }
    buy
}

fn apply_need_shares(buy: &mut GoodsVec, need: &PopNeed, qty: f64, shares: &[(GoodIdx, f64)]) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use vic3_defs::{BuyPackage, GameDefs, Good, NeedEntry, PopNeed};

    fn heating_defs() -> GameDefs {
        let heat = NeedIdx::from_usize(0);
        let mut defs = GameDefs {
            price_range: 0.75,
            goods_order: vec!["grain".into(), "wood".into(), "coal".into()],
            needs_order: vec!["popneed_heating".into()],
            pop_needs: vec![PopNeed {
                id: "popneed_heating".into(),
                default_good: Some(GoodIdx::from_usize(1)),
                entries: vec![
                    NeedEntry {
                        good: GoodIdx::from_usize(1),
                        weight: 1.0,
                        min_supply_share: 0.0,
                        max_supply_share: 0.5,
                    },
                    NeedEntry {
                        good: GoodIdx::from_usize(2),
                        weight: 2.0,
                        min_supply_share: 0.1,
                        max_supply_share: 1.0,
                    },
                ],
            }],
            ..GameDefs::default()
        };
        defs.goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
                traded_quantity: 10.0,
                texture: None,
            },
        );
        defs.goods.insert(
            "coal".into(),
            Good {
                id: "coal".into(),
                base_price: 30.0,
                traded_quantity: 6.0,
                texture: None,
            },
        );
        let mut needs1 = NeedsVec::zeros(1);
        needs1[heat] = 15.0;
        defs.buy_packages.insert(
            1,
            BuyPackage {
                wealth: 1,
                political_strength: 0.03,
                needs: needs1,
            },
        );
        let mut needs2 = NeedsVec::zeros(1);
        needs2[heat] = 17.0;
        defs.buy_packages.insert(
            2,
            BuyPackage {
                wealth: 2,
                political_strength: 0.04,
                needs: needs2,
            },
        );
        defs.rebuild_package_ladder();
        defs
    }

    fn pop(size: f64, wealth: u8, wages: f64) -> WorldPop {
        WorldPop {
            state: None,
            size,
            wealth,
            wages,
            culture: None,
            profession: None,
        }
    }

    #[test]
    fn unit_baskets_match_size_scaled_integer_wealth() {
        let defs = heating_defs();
        let n = defs.goods_order.len();
        let base = GoodsVec::from_vec(vec![20.0, 20.0, 30.0]);
        let sell = GoodsVec::from_vec(vec![0.0, 10.0, 10.0]);
        let shares = NeedShares::from_sell(&defs, &sell);
        let units = UnitBaskets::from_shares(&defs, &base, &shares);
        let small = pop(POP_SCALE, 1, 0.0);
        let big = pop(2.0 * POP_SCALE, 1, 0.0);
        let mut a = GoodsVec::zeros(n);
        let mut b = GoodsVec::zeros(n);
        add_pop_from_units(&mut a, &small, &base, &base, &units, 1.0);
        add_pop_from_units(&mut b, &big, &base, &base, &units, 1.0);
        for (left, right) in a.as_slice().iter().zip(b.as_slice()) {
            assert!((left * 2.0 - right).abs() < 1e-9);
        }
    }

    #[test]
    fn wage_bins_match_per_pop_add() {
        let defs = heating_defs();
        let n = defs.goods_order.len();
        let base = GoodsVec::from_vec(vec![20.0, 20.0, 30.0]);
        let local = GoodsVec::from_vec(vec![20.0, 40.0, 30.0]);
        let sell = GoodsVec::from_vec(vec![0.0, 10.0, 10.0]);
        let shares = NeedShares::from_sell(&defs, &sell);
        let units = UnitBaskets::from_shares(&defs, &base, &shares);
        let pops = [
            pop(POP_SCALE, 1, 1.0),
            pop(2.0 * POP_SCALE, 1, 1.0),
            pop(POP_SCALE, 2, 1.0),
        ];
        let mut per_pop = GoodsVec::zeros(n);
        for p in &pops {
            add_pop_from_units(&mut per_pop, p, &local, &base, &units, 1.0);
        }
        let bins = wage_bins_from_pops(&pops);
        assert_eq!(bins.len(), 2);
        let mut binned = GoodsVec::zeros(n);
        add_wage_bins(&mut binned, &bins, &local, &base, &units, 1.0);
        for (left, right) in per_pop.as_slice().iter().zip(binned.as_slice()) {
            assert!((left - right).abs() < 1e-9);
        }
    }

    fn assert_baskets_close(left: &[NeedBasket], right: &[NeedBasket]) {
        assert_eq!(left.len(), right.len());
        for (a, b) in left.iter().zip(right) {
            assert_eq!(a.need_idx, b.need_idx);
            assert!((a.package_value - b.package_value).abs() < 1e-9);
            assert_eq!(a.goods.len(), b.goods.len());
            for ((g0, q0), (g1, q1)) in a.goods.iter().zip(&b.goods) {
                assert_eq!(g0, g1);
                assert!((q0 - q1).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn unit_need_baskets_match_direct_integer_and_lerp() {
        let defs = heating_defs();
        let base = GoodsVec::from_vec(vec![20.0, 20.0, 30.0]);
        let local = GoodsVec::from_vec(vec![20.0, 40.0, 30.0]);
        let sell = GoodsVec::from_vec(vec![0.0, 10.0, 10.0]);
        let shares = NeedShares::from_sell(&defs, &sell);
        let units = UnitBaskets::from_shares(&defs, &base, &shares);
        let need_units = UnitNeedBaskets::from_shares(&defs, &base, &shares);
        for (size, wealth, wages) in [
            (POP_SCALE, 1u8, 0.0),
            (2.0 * POP_SCALE, 1, 0.0),
            (POP_SCALE, 2, 0.0),
            (POP_SCALE, 1, 1.0),
            (1.5 * POP_SCALE, 2, 1.0),
        ] {
            let household = pop(size, wealth, wages);
            let cached = pop_need_baskets(&household, &local, &base, &units, &need_units);
            let wealth_f = if wages <= 0.0 {
                f64::from(wealth)
            } else {
                units.continuous_wealth(wealth, &local, &base)
            };
            let direct = need_baskets_at(wealth_f, size / POP_SCALE, &defs, &base, &shares);
            assert_baskets_close(&cached, &direct);
            let compact = need_units.compact_for(wealth_f, size / POP_SCALE, &local);
            assert_eq!(compact.len(), cached.len());
            for (row, basket) in compact.iter().zip(&cached) {
                assert_eq!(row.need_idx, basket.need_idx);
                assert!((row.package_value - basket.package_value).abs() < 1e-9);
                assert_eq!(row.goods.len(), basket.goods.len());
                for (&(good, quantity, value), &(expected_good, expected_qty)) in
                    row.goods.iter().zip(&basket.goods)
                {
                    assert_eq!(good, expected_good);
                    assert!((quantity - expected_qty).abs() < 1e-9);
                    assert!((value - expected_qty * local[good]).abs() < 1e-9);
                }
            }
        }
    }
}
