//! Shared Exact pushdown classification for providers.

use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};

use crate::filter::{between_str, classify_eq_str, classify_eq_u32, classify_range_str, Pred};

pub struct PushSupport {
    pub eq_u32: &'static [&'static str],
    pub eq_str: &'static [&'static str],
    /// Columns backed by a BTreeMap (Exact range allowed).
    pub range_str: &'static [&'static str],
}

impl PushSupport {
    pub fn classify(&self, filters: &[&Expr]) -> Vec<TableProviderFilterPushDown> {
        filters
            .iter()
            .map(|f| {
                if classify_eq_u32(f, self.eq_u32).is_some()
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

    pub fn collect_preds(&self, filters: &[Expr]) -> Vec<Pred> {
        let mut out = Vec::new();
        for f in filters {
            if let Some((column, value)) = classify_eq_u32(f, self.eq_u32) {
                out.push(Pred::EqU32 { column, value });
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
        Pred::EqStr { .. } | Pred::RangeStr { .. } | Pred::EqU32 { .. } => true,
    })
}
