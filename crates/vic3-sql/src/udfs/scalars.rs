//! Scalar diagnostics: `good_price(good)`, `army_power()`, `player_tag()`,
//! `is_underemployed(state_id)` (`docs/sql.md`).
//!
//! `good_price` returns NULL for a NULL arg or unknown good id. `army_power`
//! returns NULL when there is no `player_tag`. When a player country is bound
//! but save IR has no army power projection fields, the UDF **errors** (and
//! logs) instead of returning a silent NULL or false zero. `player_tag`
//! returns the bound world's played tag, or NULL if unset (no first-country
//! fallback — matches `army_power` honesty). `is_underemployed` is true when
//! the bound session has an [`AlertKind::Underemployed`] alert for that
//! `state_id` (same detector as `alerts()`); NULL arg → NULL.

use std::sync::Arc;

use datafusion::arrow::array::{Array, BooleanBuilder, Float64Builder};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::cast::as_string_array;
use datafusion::common::{exec_err, Result as DfResult, ScalarValue};
use datafusion::logical_expr::{create_udf, ColumnarValue, Volatility};
use datafusion::prelude::SessionContext;
use vic3_prices::AlertKind;

use crate::binding::SessionBinding;
use crate::SqlError;

/// Register scalar diagnostics as Stable UDFs over the bound session.
pub fn register(ctx: &SessionContext, binding: Arc<SessionBinding>) -> Result<(), SqlError> {
    let price_binding = Arc::clone(&binding);
    ctx.register_udf(create_udf(
        "good_price",
        vec![DataType::Utf8],
        DataType::Float64,
        Volatility::Stable,
        Arc::new(move |args| good_price_invoke(price_binding.as_ref(), args)),
    ));

    let army_binding = Arc::clone(&binding);
    ctx.register_udf(create_udf(
        "army_power",
        vec![],
        DataType::Float64,
        Volatility::Stable,
        Arc::new(move |_args| army_power_invoke(army_binding.as_ref())),
    ));

    let tag_binding = Arc::clone(&binding);
    ctx.register_udf(create_udf(
        "player_tag",
        vec![],
        DataType::Utf8,
        Volatility::Stable,
        Arc::new(move |_args| player_tag_invoke(tag_binding.as_ref())),
    ));

    let under_binding = binding;
    ctx.register_udf(create_udf(
        "is_underemployed",
        vec![DataType::Int64],
        DataType::Boolean,
        Volatility::Stable,
        Arc::new(move |args| is_underemployed_invoke(under_binding.as_ref(), args)),
    ));
    Ok(())
}

/// Market price for `good`, or NULL when the id is missing / the arg is NULL.
fn good_price_invoke(binding: &SessionBinding, args: &[ColumnarValue]) -> DfResult<ColumnarValue> {
    let arg = args.first().ok_or_else(|| {
        datafusion::common::DataFusionError::Internal("good_price expects one argument".into())
    })?;
    match arg {
        ColumnarValue::Scalar(sv) => {
            let price = utf8_scalar(sv)?.and_then(|id| lookup_price(binding, &id));
            Ok(ColumnarValue::Scalar(ScalarValue::Float64(price)))
        }
        ColumnarValue::Array(arr) => {
            // Fast path for Utf8; otherwise per-row ScalarValue decode (LargeUtf8 / views).
            if let Ok(strings) = as_string_array(arr.as_ref()) {
                let mut out = Float64Builder::with_capacity(strings.len());
                for i in 0..strings.len() {
                    if strings.is_null(i) {
                        out.append_null();
                    } else {
                        append_price(&mut out, binding, strings.value(i));
                    }
                }
                return Ok(ColumnarValue::Array(Arc::new(out.finish())));
            }
            let mut out = Float64Builder::with_capacity(arr.len());
            for i in 0..arr.len() {
                let sv = ScalarValue::try_from_array(arr.as_ref(), i)?;
                match utf8_scalar(&sv)? {
                    Some(id) => append_price(&mut out, binding, &id),
                    None => out.append_null(),
                }
            }
            Ok(ColumnarValue::Array(Arc::new(out.finish())))
        }
    }
}

fn utf8_scalar(sv: &ScalarValue) -> DfResult<Option<String>> {
    match sv {
        ScalarValue::Utf8(v) | ScalarValue::LargeUtf8(v) | ScalarValue::Utf8View(v) => {
            Ok(v.clone())
        }
        ScalarValue::Null => Ok(None),
        other => exec_err!("good_price: expected UTF8, got {other}"),
    }
}

fn append_price(out: &mut Float64Builder, binding: &SessionBinding, good: &str) {
    match lookup_price(binding, good) {
        Some(p) => out.append_value(p),
        None => out.append_null(),
    }
}

fn lookup_price(binding: &SessionBinding, good: &str) -> Option<f64> {
    binding
        .prices
        .goods
        .iter()
        .find(|g| g.good_name == good)
        .map(|g| g.price)
}

fn army_power_invoke(binding: &SessionBinding) -> DfResult<ColumnarValue> {
    match army_power(binding)? {
        Some(power) => Ok(ColumnarValue::Scalar(ScalarValue::Float64(Some(power)))),
        None => Ok(ColumnarValue::Scalar(ScalarValue::Float64(None))),
    }
}

/// Player country's projection when known.
///
/// - [`None`] (SQL NULL) — no `player_tag` on the bound world
/// - [`Err`] — player tag set but country missing, or projection IR absent
/// - [`Some`] — known finite projection
fn army_power(binding: &SessionBinding) -> DfResult<Option<f64>> {
    let Some(tag) = binding.world.player_tag.as_deref() else {
        return Ok(None);
    };
    let Some(country) = binding.world.countries.iter().find(|c| c.tag == tag) else {
        tracing::error!(
            player_tag = tag,
            "army_power(): player_tag set but no matching countries row"
        );
        return exec_err!(
            "army_power(): player_tag {tag:?} has no matching country in the bound session"
        );
    };
    match country.army_power_projection {
        Some(power) => Ok(Some(power)),
        None => {
            tracing::error!(
                player_tag = tag,
                "army_power(): save IR has no army power projection for player (refusing silent NULL/0)"
            );
            exec_err!(
                "army_power(): army power projection unknown for player {tag:?} \
                 (save IR missing cached/formation power_projection; not a zero army)"
            )
        }
    }
}

fn player_tag_invoke(binding: &SessionBinding) -> DfResult<ColumnarValue> {
    Ok(ColumnarValue::Scalar(ScalarValue::Utf8(
        binding.world.player_tag.clone(),
    )))
}

/// True when [`AlertKind::Underemployed`] fires for `state_id` on this bind.
fn is_underemployed_invoke(
    binding: &SessionBinding,
    args: &[ColumnarValue],
) -> DfResult<ColumnarValue> {
    let arg = args.first().ok_or_else(|| {
        datafusion::common::DataFusionError::Internal(
            "is_underemployed expects one argument".into(),
        )
    })?;
    match arg {
        ColumnarValue::Scalar(sv) => {
            let flag = state_id_scalar(sv)?.map(|sid| state_is_underemployed(binding, sid));
            Ok(ColumnarValue::Scalar(ScalarValue::Boolean(flag)))
        }
        ColumnarValue::Array(arr) => {
            let mut out = BooleanBuilder::with_capacity(arr.len());
            for i in 0..arr.len() {
                let sv = ScalarValue::try_from_array(arr.as_ref(), i)?;
                match state_id_scalar(&sv)? {
                    Some(sid) => out.append_value(state_is_underemployed(binding, sid)),
                    None => out.append_null(),
                }
            }
            Ok(ColumnarValue::Array(Arc::new(out.finish())))
        }
    }
}

fn state_is_underemployed(binding: &SessionBinding, state_id: u32) -> bool {
    // Lean alerts (no mitigations) — same Underemployed detector as `alerts()`.
    binding
        .alerts(false)
        .alerts
        .iter()
        .any(|alert| alert.kind == AlertKind::Underemployed && alert.state_id == Some(state_id))
}

fn state_id_scalar(sv: &ScalarValue) -> DfResult<Option<u32>> {
    match sv {
        ScalarValue::Null => Ok(None),
        ScalarValue::Int64(None)
        | ScalarValue::Int32(None)
        | ScalarValue::UInt32(None)
        | ScalarValue::UInt64(None) => Ok(None),
        ScalarValue::Int64(Some(v)) if *v >= 0 => u32::try_from(*v).map(Some).map_err(|_| {
            datafusion::common::DataFusionError::Execution(format!(
                "is_underemployed: state_id {v} out of u32 range"
            ))
        }),
        ScalarValue::Int32(Some(v)) if *v >= 0 => Ok(Some(*v as u32)),
        ScalarValue::UInt32(Some(v)) => Ok(Some(*v)),
        ScalarValue::UInt64(Some(v)) => u32::try_from(*v).map(Some).map_err(|_| {
            datafusion::common::DataFusionError::Execution(format!(
                "is_underemployed: state_id {v} out of u32 range"
            ))
        }),
        ScalarValue::Int64(Some(_)) | ScalarValue::Int32(Some(_)) => {
            exec_err!("is_underemployed: state_id must be non-negative")
        }
        other => exec_err!("is_underemployed: expected integer state_id, got {other}"),
    }
}
