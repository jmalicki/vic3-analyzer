//! Normalize substitution shares against indexed sell orders.
//!
//! Vic3 maps each pop need onto goods with `weight` and
//! `min_supply_share` / `max_supply_share`. The price loop uses
//! [`substitution_shares`] so consumption respects invariant **I4**
//! ([`docs/invariants.md`](../../../docs/invariants.md),
//! [`docs/prices.md`](../../../docs/prices.md)).

use crate::{GoodId, GoodsVec, NeedEntry, PopNeed};

/// Clamp a market sell-order share to a need entry's substitution caps (I4).
///
/// Vic3 applies `min_supply_share` / `max_supply_share` to the good's share of
/// sell orders before weighting. If `min > max`, the bounds are swapped so
/// clamp is always defined.
pub fn clamp_supply_share(raw_share: f64, entry: &NeedEntry) -> f64 {
    let lo = entry.min_supply_share.min(entry.max_supply_share);
    let hi = entry.min_supply_share.max(entry.max_supply_share);
    raw_share.clamp(lo, hi)
}

/// Unnormalized substitution weight: `weight * clamp(sell_share, min, max)`.
pub fn substitution_weight(entry: &NeedEntry, raw_sell_share: f64) -> f64 {
    entry.weight * clamp_supply_share(raw_sell_share, entry)
}

/// Normalized substitution shares for `need` given dense sell-order quantities.
///
/// Per-entry raw share is that good's sell quantity over the need's total sell
/// among its entries (missing / zero sell → raw share `0`). Shares sum to `1`
/// when any unnormalized weight is positive; otherwise every returned share is
/// `0`.
pub fn substitution_shares(need: &PopNeed, sell_orders: &GoodsVec) -> Vec<(GoodId, f64)> {
    let total_sell: f64 = need
        .entries
        .iter()
        .map(|e| sell_orders[e.good].max(0.0))
        .sum();
    let weights: Vec<(GoodId, f64)> = need
        .entries
        .iter()
        .map(|entry| {
            let raw = if total_sell > 0.0 {
                sell_orders[entry.good].max(0.0) / total_sell
            } else {
                0.0
            };
            (entry.good, substitution_weight(entry, raw))
        })
        .collect();
    let total: f64 = weights.iter().map(|(_, w)| *w).sum();
    weights
        .into_iter()
        .map(|(good, w)| {
            let share = if total > 0.0 { w / total } else { 0.0 };
            (good, share)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NeedEntry;
    use proptest::prelude::*;

    const EPS: f64 = 1e-9;

    fn arb_share_bound() -> impl Strategy<Value = f64> {
        0.0f64..=1.0
    }

    fn arb_weight() -> impl Strategy<Value = f64> {
        0.1f64..=8.0
    }

    fn arb_raw_share() -> impl Strategy<Value = f64> {
        0.0f64..=1.5
    }

    #[derive(Clone, Debug)]
    struct SynthRow {
        min: f64,
        max: f64,
        weight: f64,
        raw: f64,
    }

    fn arb_row() -> impl Strategy<Value = SynthRow> {
        (
            arb_share_bound(),
            arb_share_bound(),
            arb_weight(),
            arb_raw_share(),
        )
            .prop_map(|(a, b, weight, raw)| SynthRow {
                min: a.min(b),
                max: a.max(b),
                weight,
                raw,
            })
    }

    fn need_from_rows(rows: &[SynthRow]) -> (PopNeed, GoodsVec) {
        let entries = rows
            .iter()
            .enumerate()
            .map(|(i, row)| NeedEntry {
                good: GoodId::from_usize(i),
                weight: row.weight,
                min_supply_share: row.min,
                max_supply_share: row.max,
            })
            .collect();
        // `raw` is treated as a sell quantity; shares are quantity / total.
        let sell = GoodsVec::from_vec(rows.iter().map(|row| row.raw.max(0.0)).collect());
        (
            PopNeed {
                id: "synth".into(),
                default_good: None,
                entries,
            },
            sell,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// I4: substitution respects `min_supply_share` / `max_supply_share`.
        #[test]
        fn i4_substitution_respects_min_max_supply_share(
            rows in prop::collection::vec(arb_row(), 1..8)
        ) {
            let (need, sell_orders) = need_from_rows(&rows);
            let total_sell: f64 = sell_orders.as_slice().iter().map(|s| s.max(0.0)).sum();
            for entry in &need.entries {
                let raw = if total_sell > 0.0 {
                    sell_orders[entry.good].max(0.0) / total_sell
                } else {
                    0.0
                };
                let clamped = clamp_supply_share(raw, entry);
                prop_assert!(clamped + EPS >= entry.min_supply_share.min(entry.max_supply_share));
                prop_assert!(clamped - EPS <= entry.min_supply_share.max(entry.max_supply_share));
                let w = substitution_weight(entry, raw);
                let expected = entry.weight * clamped;
                prop_assert!((w - expected).abs() < EPS);
            }

            let shares = substitution_shares(&need, &sell_orders);
            let share_sum: f64 = shares.iter().map(|(_, s)| *s).sum();
            let weight_sum: f64 = need
                .entries
                .iter()
                .map(|e| {
                    let raw = if total_sell > 0.0 {
                        sell_orders[e.good].max(0.0) / total_sell
                    } else {
                        0.0
                    };
                    substitution_weight(e, raw)
                })
                .sum();
            if weight_sum > 0.0 {
                prop_assert!((share_sum - 1.0).abs() < 1e-6);
            } else {
                prop_assert!(share_sum.abs() < EPS);
            }
        }

        /// Raising a good's sell share past `max_supply_share` must not change its weight.
        #[test]
        fn i4_max_supply_share_caps_weight(
            min in arb_share_bound(),
            max in arb_share_bound(),
            weight in arb_weight(),
            extra in 0.0f64..=2.0
        ) {
            let lo = min.min(max);
            let hi = min.max(max);
            let entry = NeedEntry {
                good: GoodId::from_usize(0),
                weight,
                min_supply_share: lo,
                max_supply_share: hi,
            };
            let at_cap = substitution_weight(&entry, hi);
            let above_cap = substitution_weight(&entry, hi + extra);
            prop_assert!((at_cap - above_cap).abs() < EPS);
        }
    }
}
