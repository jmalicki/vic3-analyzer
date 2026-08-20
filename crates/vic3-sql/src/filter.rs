//! Predicate helpers for Exact pushdown (`docs/sql.md`).
//!
//! Decodes DataFusion filter [`Expr`]s into [`Pred`] for providers that
//! advertise Exact equality / range via [`crate::providers::pushdown`].

use datafusion::common::ScalarValue;
use datafusion::logical_expr::{BinaryExpr, Expr, Operator};

/// Bound for an inclusive/exclusive string or integer range endpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum Bound<T> {
    Inclusive(T),
    Exclusive(T),
}

/// Equality / range predicates we understand for pushdown.
#[derive(Debug, Clone, PartialEq)]
pub enum Pred {
    EqU32 {
        column: String,
        value: u32,
    },
    EqStr {
        column: String,
        value: String,
    },
    RangeStr {
        column: String,
        low: Option<Bound<String>>,
        high: Option<Bound<String>>,
    },
}

pub fn classify_eq_u32(expr: &Expr, columns: &[&str]) -> Option<(String, u32)> {
    match_binary(expr, Operator::Eq, |col, lit| {
        if !columns.contains(&col.as_str()) {
            return None;
        }
        scalar_u32(lit).map(|v| (col, v))
    })
}

pub fn classify_eq_str(expr: &Expr, columns: &[&str]) -> Option<(String, String)> {
    match_binary(expr, Operator::Eq, |col, lit| {
        if !columns.contains(&col.as_str()) {
            return None;
        }
        scalar_utf8(lit).map(|v| (col, v))
    })
}

pub fn classify_range_str(expr: &Expr, columns: &[&str]) -> Option<Pred> {
    let (op, left, right) = match expr {
        Expr::BinaryExpr(BinaryExpr { left, op, right }) => (op, left.as_ref(), right.as_ref()),
        _ => return None,
    };
    let (col, lit, reverse) = match (column_name(left), as_scalar(right)) {
        (Some(c), Some(s)) => (c, s, false),
        _ => match (column_name(right), as_scalar(left)) {
            (Some(c), Some(s)) => (c, s, true),
            _ => return None,
        },
    };
    if !columns.contains(&col.as_str()) {
        return None;
    }
    let value = scalar_utf8(lit)?;
    let (low, high) = match (op, reverse) {
        (Operator::Gt, false) | (Operator::Lt, true) => (Some(Bound::Exclusive(value)), None),
        (Operator::GtEq, false) | (Operator::LtEq, true) => (Some(Bound::Inclusive(value)), None),
        (Operator::Lt, false) | (Operator::Gt, true) => (None, Some(Bound::Exclusive(value))),
        (Operator::LtEq, false) | (Operator::GtEq, true) => (None, Some(Bound::Inclusive(value))),
        _ => return None,
    };
    Some(Pred::RangeStr {
        column: col,
        low,
        high,
    })
}

pub fn between_str(expr: &Expr, columns: &[&str]) -> Option<Pred> {
    let Expr::Between(between) = expr else {
        return None;
    };
    if between.negated {
        return None;
    }
    let col = column_name(between.expr.as_ref())?;
    if !columns.contains(&col.as_str()) {
        return None;
    }
    let low = as_scalar(between.low.as_ref()).and_then(scalar_utf8)?;
    let high = as_scalar(between.high.as_ref()).and_then(scalar_utf8)?;
    Some(Pred::RangeStr {
        column: col,
        low: Some(Bound::Inclusive(low)),
        high: Some(Bound::Inclusive(high)),
    })
}

fn match_binary<T>(
    expr: &Expr,
    want: Operator,
    f: impl FnOnce(String, &ScalarValue) -> Option<T>,
) -> Option<T> {
    let Expr::BinaryExpr(BinaryExpr { left, op, right }) = expr else {
        return None;
    };
    if *op != want {
        return None;
    }
    match (column_name(left), as_scalar(right)) {
        (Some(c), Some(s)) => f(c, s),
        _ => match (column_name(right), as_scalar(left)) {
            (Some(c), Some(s)) => f(c, s),
            _ => None,
        },
    }
}

fn column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Column(c) => Some(c.name.clone()),
        Expr::Alias(a) => column_name(a.expr.as_ref()),
        _ => None,
    }
}

fn as_scalar(expr: &Expr) -> Option<&ScalarValue> {
    match expr {
        Expr::Literal(v, _) => Some(v),
        _ => None,
    }
}

fn scalar_utf8(v: &ScalarValue) -> Option<String> {
    match v {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => Some(s.clone()),
        _ => None,
    }
}

fn scalar_u32(v: &ScalarValue) -> Option<u32> {
    match v {
        ScalarValue::UInt32(Some(n)) => Some(*n),
        ScalarValue::UInt64(Some(n)) if *n <= u32::MAX as u64 => Some(*n as u32),
        ScalarValue::Int64(Some(n)) if *n >= 0 && *n <= u32::MAX as i64 => Some(*n as u32),
        ScalarValue::Int32(Some(n)) if *n >= 0 => Some(*n as u32),
        _ => None,
    }
}

/// Does `key` fall inside an optional string range?
pub fn in_str_range(key: &str, low: &Option<Bound<String>>, high: &Option<Bound<String>>) -> bool {
    if let Some(b) = low {
        match b {
            Bound::Inclusive(v) if key < v.as_str() => return false,
            Bound::Exclusive(v) if key <= v.as_str() => return false,
            _ => {}
        }
    }
    if let Some(b) = high {
        match b {
            Bound::Inclusive(v) if key > v.as_str() => return false,
            Bound::Exclusive(v) if key >= v.as_str() => return false,
            _ => {}
        }
    }
    true
}
