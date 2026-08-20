//! Closed-form Vic3 market price and MAPI blend helpers.
//!
//! [`price`] / [`market_ratio`] implement the wiki formula with a documented
//! zero-order convention ([`ORDER_EPS`]). [`market_access`] → [`effective_mapi`]
//! → [`local_price`] are the Milestone-1 local blend used inside the residual
//! (pops shop locally; state orders are then access-scaled into one whole-save
//! market). Extra MAPI modifiers and overseas convoy constraints are out of
//! scope until the IR carries them.

/// Orders at or below this are treated as empty so `min(buy, sell)` never
/// divides by a numerical zero. I1–I3 are required only for strictly larger
/// positive orders.
pub const ORDER_EPS: f64 = 1e-12;

/// Base market-access price impact, before multiplying by infrastructure access.
///
/// Vanilla uses `0.75`; [`effective_mapi`] = `BASE_MAPI * market_access`.
pub const BASE_MAPI: f64 = 0.75;

/// Buy/sell imbalance used by [`price`].
///
/// # Zero orders
///
/// The wiki divisor is `min(buy, sell)`. That is zero when either side has
/// no orders, so this crate does **not** divide in that case:
///
/// | `buy` | `sell` | ratio |
/// | --- | --- | --- |
/// | both `≤ ORDER_EPS` | | `0` (no market → base price) |
/// | `buy > ORDER_EPS`, `sell ≤ ORDER_EPS` | | `+1` (shortage → clamp high) |
/// | `buy ≤ ORDER_EPS`, `sell > ORDER_EPS` | | `−1` (glut → clamp low) |
/// | both `> ORDER_EPS` | | `(buy - sell) / min(buy, sell)` |
///
/// Negative orders are treated as `0`. I1–I3 are required only away from this
/// singularity (both sides strictly positive and above [`ORDER_EPS`]).
pub fn market_ratio(buy: f64, sell: f64) -> f64 {
    let buy = buy.max(0.0);
    let sell = sell.max(0.0);
    let denom = buy.min(sell);
    if denom > ORDER_EPS {
        (buy - sell) / denom
    } else if buy <= ORDER_EPS && sell <= ORDER_EPS {
        0.0
    } else if buy > sell {
        1.0
    } else {
        -1.0
    }
}

/// Vic3 market price from buy/sell orders.
///
/// ```text
/// ratio = (buy - sell) / min(buy, sell)
/// price = base * (1 + PRICE_RANGE * clamp(ratio, -1, +1))
/// ```
///
/// See [`market_ratio`] for the zero-order convention. The clamp is part of
/// the model (I2): the result stays in
/// `[1 - price_range, 1 + price_range] * base` when `price_range ≥ 0`.
///
/// **I1:** `buy == sell` (including both zero) ⇒ `price == base`.
/// **I3:** weakly more buy with sell fixed ⇒ weakly higher price, away from
/// the clamp and the zero-order singularity.
pub fn price(base: f64, buy: f64, sell: f64, price_range: f64) -> f64 {
    let ratio = market_ratio(buy, sell).clamp(-1.0, 1.0);
    base * (1.0 + price_range * ratio)
}

/// Infrastructure-only market access in `[0, 1]`.
///
/// `clamp(infrastructure / infrastructure_usage, 0, 1)`. Missing data and zero
/// usage default to full access (`1.0`). Overseas convoy and shipping-lane
/// constraints are not represented in the current save IR.
pub fn market_access(infrastructure: Option<f64>, usage: Option<f64>) -> f64 {
    match (infrastructure, usage) {
        (Some(infrastructure), Some(usage)) if usage > 0.0 => {
            (infrastructure / usage).clamp(0.0, 1.0)
        }
        _ => 1.0,
    }
}

/// Blend a pure state price with its market price using effective MAPI.
///
/// Vic3: `local = mapi * market + (1 - mapi) * state`. Wage pops and building
/// economics in the residual use this local price; substitution shares still
/// come from **world** sell orders.
pub fn local_price(effective_mapi: f64, market_price: f64, state_price: f64) -> f64 {
    let mapi = effective_mapi.clamp(0.0, 1.0);
    mapi * market_price + (1.0 - mapi) * state_price
}

/// Infrastructure access times [`BASE_MAPI`].
pub fn effective_mapi(access: f64) -> f64 {
    BASE_MAPI * access.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const EPS: f64 = 1e-9;

    fn arb_base() -> impl Strategy<Value = f64> {
        0.1f64..=1_000.0
    }

    fn arb_price_range() -> impl Strategy<Value = f64> {
        0.05f64..=1.0
    }

    fn arb_order() -> impl Strategy<Value = f64> {
        0.0f64..=1_000.0
    }

    fn arb_positive_order() -> impl Strategy<Value = f64> {
        1e-3f64..=1_000.0
    }

    #[test]
    fn zero_orders_are_base() {
        assert!((price(20.0, 0.0, 0.0, 0.75) - 20.0).abs() < EPS);
    }

    #[test]
    fn only_buy_clamps_high() {
        let p = price(20.0, 10.0, 0.0, 0.75);
        assert!((p - 20.0 * 1.75).abs() < EPS);
    }

    #[test]
    fn only_sell_clamps_low() {
        let p = price(20.0, 0.0, 10.0, 0.75);
        assert!((p - 20.0 * 0.25).abs() < EPS);
    }

    #[test]
    fn wiki_mapi_blends_market_and_state_prices() {
        assert!((local_price(0.85, 40.0, 10.0) - 35.5).abs() < EPS);
    }

    #[test]
    fn infrastructure_caps_market_access() {
        assert_eq!(market_access(Some(45.0), Some(90.0)), 0.5);
        assert_eq!(market_access(Some(90.0), Some(45.0)), 1.0);
        assert_eq!(market_access(None, Some(45.0)), 1.0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// I1: `buy == sell` ⇒ market price = base (within ε).
        #[test]
        fn i1_equal_buy_sell_is_base_price(
            base in arb_base(),
            qty in arb_order(),
            price_range in arb_price_range(),
        ) {
            let p = price(base, qty, qty, price_range);
            prop_assert!((p - base).abs() < 1e-9 * (1.0 + base.abs()));
        }

        /// I2: prices stay in `[1 - PRICE_RANGE, 1 + PRICE_RANGE] * base`.
        #[test]
        fn i2_prices_stay_in_price_range(
            base in arb_base(),
            buy in arb_order(),
            sell in arb_order(),
            price_range in arb_price_range(),
        ) {
            let p = price(base, buy, sell, price_range);
            let lo = base * (1.0 - price_range);
            let hi = base * (1.0 + price_range);
            prop_assert!(p + EPS >= lo);
            prop_assert!(p - EPS <= hi);
        }

        /// I2 at clamp extremes (one-sided and huge imbalance).
        #[test]
        fn i2_clamp_extremes(
            base in arb_base(),
            big in 1.0f64..=1_000.0,
            price_range in arb_price_range(),
        ) {
            let high = price(base, big, 0.0, price_range);
            let low = price(base, 0.0, big, price_range);
            prop_assert!((high - base * (1.0 + price_range)).abs() < 1e-9 * (1.0 + high.abs()));
            prop_assert!((low - base * (1.0 - price_range)).abs() < 1e-9 * (1.0 + low.abs()));
            let clamped_high = price(base, 10.0 * big, big, price_range);
            let clamped_low = price(base, big, 10.0 * big, price_range);
            prop_assert!((clamped_high - base * (1.0 + price_range)).abs() < 1e-9 * (1.0 + clamped_high.abs()));
            prop_assert!((clamped_low - base * (1.0 - price_range)).abs() < 1e-9 * (1.0 + clamped_low.abs()));
        }

        /// I3: weakly more buy (sell fixed) ⇒ weakly higher price, away from clamp.
        #[test]
        fn i3_more_buy_weakly_raises_price(
            base in arb_base(),
            sell in arb_positive_order(),
            t1 in 0.05f64..=0.95,
            t2 in 0.05f64..=0.95,
            price_range in arb_price_range(),
        ) {
            // Unclamped open interval: buy ∈ (sell/2, 2*sell).
            let lo = sell * 0.51;
            let hi = sell * 1.99;
            let buy_a = lo + t1.min(t2) * (hi - lo);
            let buy_b = lo + t1.max(t2) * (hi - lo);
            let p_a = price(base, buy_a, sell, price_range);
            let p_b = price(base, buy_b, sell, price_range);
            prop_assert!(p_b + EPS >= p_a);
        }
    }
}
