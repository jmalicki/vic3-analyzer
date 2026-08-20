//! Plan-time literal extraction for diagnostics TVF arguments.
//!
//! DataFusion UDTFs resolve arguments at `call` time; we only accept literals
//! so providers can materialize a fixed batch (no deferred expression eval).

use datafusion::common::{plan_err, Result as DfResult, ScalarValue};
use datafusion::logical_expr::Expr;

/// UTF-8 / LargeUtf8 / Utf8View literal, or `NULL` → [`None`].
///
/// Used by `shortage_analysis(good)` where `NULL` means “all scarce goods”.
pub fn literal_utf8(expr: &Expr, arg_name: &str) -> DfResult<Option<String>> {
    match expr {
        Expr::Literal(ScalarValue::Utf8(v), _) | Expr::Literal(ScalarValue::LargeUtf8(v), _) => {
            Ok(v.clone())
        }
        Expr::Literal(ScalarValue::Utf8View(v), _) => Ok(v.clone()),
        Expr::Literal(ScalarValue::Null, _) => Ok(None),
        other => plan_err!("{arg_name} must be a string or NULL literal, got {other}"),
    }
}

/// Non-null integer literal widened to `i64` (Int32/64, UInt32/64).
pub fn literal_i64(expr: &Expr, arg_name: &str) -> DfResult<i64> {
    match expr {
        Expr::Literal(ScalarValue::Int64(Some(v)), _) => Ok(*v),
        Expr::Literal(ScalarValue::Int32(Some(v)), _) => Ok(i64::from(*v)),
        Expr::Literal(ScalarValue::UInt32(Some(v)), _) => Ok(i64::from(*v)),
        Expr::Literal(ScalarValue::UInt64(Some(v)), _) => i64::try_from(*v).map_err(|_| {
            datafusion::common::DataFusionError::Plan(format!("{arg_name} out of range"))
        }),
        other => plan_err!("{arg_name} must be an integer literal, got {other}"),
    }
}
