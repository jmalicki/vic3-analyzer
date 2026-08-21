//! SQL scalars and TVFs (`docs/sql.md`): diagnostics + planning.
//!
//! # Registration
//!
//! [`register`] installs everything on the bound [`SessionBinding`]. Called from
//! [`crate::SqlEngine::bind`] and again after [`crate::SqlEngine::use_save`].
//! Catalog-only engines have no UDFs until the first successful `use_save`.
//!
//! # Argument contracts
//!
//! TVFs resolve args at UDTF `call` time. Only **plan-time literals** are
//! accepted ([`args`], plus internal literal helpers for planning TVFs)
//! so providers can materialize a fixed batch.
//!
//! | Function | Args | NULL / defaults |
//! | --- | --- | --- |
//! | `good_price(good)` | Utf8 (runtime columnar OK) | NULL arg or unknown id → NULL |
//! | `army_power()` | none | no `player_tag` → NULL |
//! | `player_tag()` | none | no `player_tag` on world → NULL |
//! | `is_underemployed(state_id)` | Int64 (runtime columnar OK) | NULL arg → NULL; else Underemployed alert for state |
//! | `alerts([scope])` | optional `'all'` literal | zero-arg → player-scoped; `'all'` → full save |
//! | `suggest_mitigations([scope])` | optional `'player'` / `'all'` | zero-arg / `'player'` → player-scoped; `'all'` → full save |
//! | `shortage_analysis(good)` | Utf8 **or** NULL literal | NULL → all scarce-good alerts |
//! | `building_staffing(state_id)` | non-null int | NULL / non-literal → plan error |
//! | `plan(goal [, max_days [, label]])` | non-null str / non-neg int / optional str | `max_days` default `3650`; `label` accepted, omitted from rows |
//! | `gaps(goal)` | non-null string | NULL → plan error |
//!
//! # Modules
//!
//! | Module | Surface |
//! | --- | --- |
//! | [`scalars`] | `good_price`, `army_power`, `player_tag`, `is_underemployed` |
//! | [`alerts`] | `alerts()` / `alerts('all')` |
//! | [`suggest`] | `suggest_mitigations()` / `('player')` / `('all')` |
//! | [`shortage`] | `shortage_analysis(good)` |
//! | [`staffing`] | `building_staffing(state_id)` |
//! | [`plan`] | `plan(...)` → A\* steps |
//! | [`gaps`] | `gaps(goal)` → atom status |
//! | [`args`] | Literal extractors for diagnostics TVFs |

pub mod alerts;
pub mod args;
pub mod gaps;
pub mod plan;
pub mod scalars;
pub mod shortage;
pub mod staffing;
pub mod suggest;

use std::sync::Arc;

use datafusion::common::{plan_err, Result as DfResult, ScalarValue};
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;

use crate::binding::SessionBinding;
use crate::SqlError;

pub use gaps::GapsTvf;
pub use plan::PlanTvf;

/// Register diagnostics scalars/TVFs and planning TVFs on `ctx`.
///
/// # Errors
///
/// [`SqlError`] if scalar UDF registration fails (TVF registration is infallible
/// at the DataFusion API layer; bad calls fail at query plan time).
pub fn register(ctx: &SessionContext, binding: Arc<SessionBinding>) -> Result<(), SqlError> {
    scalars::register(ctx, Arc::clone(&binding))?;
    ctx.register_udtf(
        "alerts",
        Arc::new(alerts::AlertsTvf::new(Arc::clone(&binding))),
    );
    ctx.register_udtf(
        "suggest_mitigations",
        Arc::new(suggest::SuggestMitigationsTvf::new(Arc::clone(&binding))),
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

/// Non-null UTF-8 string literal (planning TVFs).
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

/// Non-null non-negative integer literal widened to `u32` (`plan` max_days).
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
