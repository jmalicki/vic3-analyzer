//! SQL scalars and TVFs (`docs/sql.md`): diagnostics + planning.

mod alerts;
mod args;
mod gaps;
mod plan;
mod scalars;
mod shortage;
mod staffing;

use std::sync::Arc;

use datafusion::common::{plan_err, Result as DfResult, ScalarValue};
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;

use crate::binding::SessionBinding;
use crate::SqlError;

pub use gaps::GapsTvf;
pub use plan::PlanTvf;

/// Register diagnostics scalars/TVFs and planning TVFs on `ctx`.
pub fn register(ctx: &SessionContext, binding: Arc<SessionBinding>) -> Result<(), SqlError> {
    scalars::register(ctx, Arc::clone(&binding))?;
    ctx.register_udtf(
        "alerts",
        Arc::new(alerts::AlertsTvf::new(Arc::clone(&binding))),
    );
    ctx.register_udtf(
        "shortage_analysis",
        Arc::new(shortage::ShortageAnalysisTvf::new(Arc::clone(&binding))),
    );
    ctx.register_udtf(
        "building_staffing",
        Arc::new(staffing::BuildingStaffingTvf::new(Arc::clone(&binding))),
    );
    ctx.register_udtf("plan", Arc::new(PlanTvf::new(Arc::clone(&binding))));
    ctx.register_udtf("gaps", Arc::new(GapsTvf::new(binding)));
    Ok(())
}

pub(crate) fn literal_str(expr: &Expr, arg_index: usize) -> DfResult<String> {
    match expr {
        Expr::Literal(scalar, _) => match scalar.try_as_str() {
            Some(Some(s)) => Ok(s.to_string()),
            Some(None) => plan_err!("argument #{} must be a non-null string", arg_index),
            None => plan_err!(
                "argument #{} must be a string literal, got {:?}",
                arg_index,
                scalar.data_type()
            ),
        },
        other => plan_err!(
            "argument #{} must be a string literal, got {other:?}",
            arg_index
        ),
    }
}

pub(crate) fn literal_u32(expr: &Expr, arg_index: usize) -> DfResult<u32> {
    match expr {
        Expr::Literal(scalar, _) => match scalar {
            ScalarValue::UInt32(Some(v)) => Ok(*v),
            ScalarValue::UInt64(Some(v)) => u32::try_from(*v).map_err(|_| {
                datafusion::common::DataFusionError::Plan(format!(
                    "argument #{arg_index} max_days {v} does not fit in u32"
                ))
            }),
            ScalarValue::Int32(Some(v)) if *v >= 0 => Ok(*v as u32),
            ScalarValue::Int64(Some(v)) if *v >= 0 => u32::try_from(*v).map_err(|_| {
                datafusion::common::DataFusionError::Plan(format!(
                    "argument #{arg_index} max_days {v} does not fit in u32"
                ))
            }),
            other => plan_err!(
                "argument #{} must be a non-negative integer, got {:?}",
                arg_index,
                other.data_type()
            ),
        },
        other => plan_err!(
            "argument #{} must be an integer literal, got {other:?}",
            arg_index
        ),
    }
}
