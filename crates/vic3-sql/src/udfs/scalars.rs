//! Scalar diagnostics: `good_price(good)`, `army_power()`.

use std::sync::Arc;

use datafusion::arrow::array::{Array, Float64Builder};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::cast::as_string_array;
use datafusion::common::{exec_err, Result as DfResult, ScalarValue};
use datafusion::logical_expr::{create_udf, ColumnarValue, Volatility};
use datafusion::prelude::SessionContext;

use crate::binding::SessionBinding;
use crate::SqlError;

pub fn register(ctx: &SessionContext, binding: Arc<SessionBinding>) -> Result<(), SqlError> {
    let price_binding = Arc::clone(&binding);
    ctx.register_udf(create_udf(
        "good_price",
        vec![DataType::Utf8],
        DataType::Float64,
        Volatility::Stable,
        Arc::new(move |args| good_price_invoke(price_binding.as_ref(), args)),
    ));

    let army_binding = binding;
    ctx.register_udf(create_udf(
        "army_power",
        vec![],
        DataType::Float64,
        Volatility::Stable,
        Arc::new(move |_args| {
            Ok(ColumnarValue::Scalar(ScalarValue::Float64(army_power(
                army_binding.as_ref(),
            ))))
        }),
    ));
    Ok(())
}

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
        .find(|g| g.id == good)
        .map(|g| g.price)
}

/// Player country's army power projection when known; otherwise `None`.
fn army_power(binding: &SessionBinding) -> Option<f64> {
    let tag = binding.world.player_tag.as_deref()?;
    binding
        .world
        .countries
        .iter()
        .find(|c| c.tag == tag)
        .map(|c| c.army_power_projection)
}
