//! Arrow List builders for TEXT[] and good-IO structs.

use std::sync::Arc;

use datafusion::arrow::array::{ArrayRef, Float64Builder, ListBuilder, StringBuilder, StructArray};
use datafusion::arrow::datatypes::{DataType, Field};
use vic3_prices::GoodFlow;

use crate::binding::SessionBinding;
use crate::schema::good_io_struct_type;

pub fn text_list_column(rows: &[Vec<String>]) -> ArrayRef {
    let mut builder = ListBuilder::new(StringBuilder::new());
    for row in rows {
        for v in row {
            builder.values().append_value(v);
        }
        builder.append(true);
    }
    Arc::new(builder.finish())
}

pub fn good_io_list_column(binding: &SessionBinding, rows: &[Vec<GoodFlow>]) -> ArrayRef {
    let fields = match good_io_struct_type() {
        DataType::Struct(f) => f,
        _ => unreachable!(),
    };
    let mut good = StringBuilder::new();
    let mut good_name = StringBuilder::new();
    let mut qty = Float64Builder::new();
    let mut offsets = vec![0i32];
    let mut len = 0i32;
    for row in rows {
        for flow in row {
            good.append_value(&flow.good_id);
            match binding.good_name(&flow.good_id) {
                Some(n) => good_name.append_value(n),
                None => good_name.append_null(),
            }
            qty.append_value(flow.quantity);
            len += 1;
        }
        offsets.push(len);
    }
    let struct_array = StructArray::new(
        fields,
        vec![
            Arc::new(good.finish()) as ArrayRef,
            Arc::new(good_name.finish()) as ArrayRef,
            Arc::new(qty.finish()) as ArrayRef,
        ],
        None,
    );
    let list_field = Field::new(
        "item",
        DataType::Struct(struct_array.fields().clone()),
        true,
    );
    let list = datafusion::arrow::array::ListArray::new(
        Arc::new(list_field),
        datafusion::arrow::buffer::OffsetBuffer::new(offsets.into()),
        Arc::new(struct_array),
        None,
    );
    Arc::new(list)
}
