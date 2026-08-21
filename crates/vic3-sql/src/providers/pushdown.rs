//! Shared Exact pushdown classification for providers.
//!
//! Policy (`docs/sql.md`): advertise [`TableProviderFilterPushDown::Exact`] only
//! when every returned row satisfies the predicate. Hash / order-index columns
//! get Exact **equality** only; `BTreeMap` keys also get Exact **range** /
//! `BETWEEN` via [`PushSupport::range_str`].

use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};

use crate::filter::{
    between_str, classify_eq_i32, classify_eq_str, classify_eq_u32, classify_range_str, Pred,
};

/// Columns a provider can push as Exact equality and/or Exact string range.
pub struct PushSupport {
    /// `col = u32` columns (ids).
    pub eq_u32: &'static [&'static str],
    /// `col = i32` columns (e.g. alert severity).
    pub eq_i32: &'static [&'static str],
    /// `col = utf8` columns (script ids / labels with a lookup).
    pub eq_str: &'static [&'static str],
    /// Columns backed by a `BTreeMap` (Exact range allowed).
    pub range_str: &'static [&'static str],
}

impl PushSupport {
    /// Classify each filter as Exact or Unsupported for DataFusion.
    pub fn classify(&self, filters: &[&Expr]) -> Vec<TableProviderFilterPushDown> {
        filters
            .iter()
            .map(|f| {
                if classify_eq_u32(f, self.eq_u32).is_some()
                    || classify_eq_i32(f, self.eq_i32).is_some()
                    || classify_eq_str(f, self.eq_str).is_some()
                    || (classify_range_str(f, self.range_str).is_some()
                        || between_str(f, self.range_str).is_some())
                {
                    TableProviderFilterPushDown::Exact
                } else {
                    TableProviderFilterPushDown::Unsupported
                }
            })
            .collect()
    }

    /// Decode recognized predicates for in-provider row filtering.
    pub fn collect_preds(&self, filters: &[Expr]) -> Vec<Pred> {
        let mut out = Vec::new();
        for f in filters {
            if let Some((column, value)) = classify_eq_u32(f, self.eq_u32) {
                out.push(Pred::EqU32 { column, value });
                continue;
            }
            if let Some((column, value)) = classify_eq_i32(f, self.eq_i32) {
                out.push(Pred::EqI32 { column, value });
                continue;
            }
            if let Some((column, value)) = classify_eq_str(f, self.eq_str) {
                out.push(Pred::EqStr { column, value });
                continue;
            }
            if let Some(p) =
                classify_range_str(f, self.range_str).or_else(|| between_str(f, self.range_str))
            {
                out.push(p);
            }
        }
        out
    }
}

/// True if `value` satisfies every `EqU32` predicate on `column` (other preds ignored).
pub fn matches_u32(preds: &[Pred], column: &str, value: u32) -> bool {
    preds.iter().all(|p| match p {
        Pred::EqU32 {
            column: c,
            value: v,
        } if c == column => *v == value,
        Pred::EqU32 { .. } => true,
        _ => true,
    })
}

/// True if `value` satisfies every `EqI32` predicate on `column`.
pub fn matches_i32(preds: &[Pred], column: &str, value: i32) -> bool {
    preds.iter().all(|p| match p {
        Pred::EqI32 {
            column: c,
            value: v,
        } if c == column => *v == value,
        Pred::EqI32 { .. } => true,
        _ => true,
    })
}

/// True if `value` satisfies every eq/range predicate on `column`.
pub fn matches_str(preds: &[Pred], column: &str, value: &str) -> bool {
    use crate::filter::in_str_range;
    preds.iter().all(|p| match p {
        Pred::EqStr {
            column: c,
            value: v,
        } if c == column => v == value,
        Pred::RangeStr {
            column: c,
            low,
            high,
        } if c == column => in_str_range(value, low, high),
        Pred::EqStr { .. } | Pred::RangeStr { .. } | Pred::EqU32 { .. } | Pred::EqI32 { .. } => {
            true
        }
    })
}
