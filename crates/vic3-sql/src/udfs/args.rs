//! Literal argument extraction for table-valued functions.

use datafusion::common::{plan_err, Result as DfResult, ScalarValue};
use datafusion::logical_expr::Expr;

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
