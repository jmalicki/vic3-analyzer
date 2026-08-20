//! RecordBatch → JSON for MCP `query` and the Tauri Advanced Query tab.
//!
//! Shape matches `docs/mcp.md`:
//! `{ "columns": [...], "rows": [...], "row_count": N }`.

use datafusion::arrow::array::{
    Array, BooleanArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
    UInt32Array, UInt64Array,
};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::util::display::{ArrayFormatter, FormatOptions};
use serde_json::{json, Value};
use thiserror::Error;

/// Failure converting Arrow batches into the agent/UI JSON payload.
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("{0}")]
    Message(String),
}

/// Convert collected batches into the shared JSON result object.
pub fn batches_to_json(batches: &[RecordBatch]) -> Result<Value, FormatError> {
    let (columns, rows) = batches_to_rows(batches)?;
    Ok(json!({
        "columns": columns,
        "rows": rows,
        "row_count": rows.len(),
    }))
}

/// Convert collected batches into CSV text (header + rows).
pub fn batches_to_csv(batches: &[RecordBatch]) -> Result<String, FormatError> {
    let (columns, rows) = batches_to_rows(batches)?;
    let mut out = String::new();
    out.push_str(&columns.join(","));
    out.push('\n');
    for row in rows {
        let cells: Vec<String> = row.iter().map(csv_escape_json_value).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    Ok(out)
}

fn batches_to_rows(batches: &[RecordBatch]) -> Result<(Vec<String>, Vec<Vec<Value>>), FormatError> {
    if batches.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let schema = batches[0].schema();
    let columns: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let mut rows = Vec::new();
    let options = FormatOptions::default();

    for batch in batches {
        if batch.num_columns() != columns.len() {
            return Err(FormatError::Message(
                "inconsistent RecordBatch schemas in query result".into(),
            ));
        }
        for row_idx in 0..batch.num_rows() {
            let mut row = Vec::with_capacity(batch.num_columns());
            for col_idx in 0..batch.num_columns() {
                let array = batch.column(col_idx);
                row.push(cell_to_json(array.as_ref(), row_idx, &options)?);
            }
            rows.push(row);
        }
    }
    Ok((columns, rows))
}

fn cell_to_json(
    array: &dyn Array,
    row: usize,
    options: &FormatOptions,
) -> Result<Value, FormatError> {
    if array.is_null(row) {
        return Ok(Value::Null);
    }
    match array.data_type() {
        DataType::Utf8 => {
            let a = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| FormatError::Message("utf8 downcast".into()))?;
            Ok(Value::String(a.value(row).to_string()))
        }
        DataType::Boolean => {
            let a = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| FormatError::Message("bool downcast".into()))?;
            Ok(Value::Bool(a.value(row)))
        }
        DataType::Int32 => {
            let a = array
                .as_any()
                .downcast_ref::<Int32Array>()
                .ok_or_else(|| FormatError::Message("i32 downcast".into()))?;
            Ok(json!(a.value(row)))
        }
        DataType::Int64 => {
            let a = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| FormatError::Message("i64 downcast".into()))?;
            Ok(json!(a.value(row)))
        }
        DataType::UInt32 => {
            let a = array
                .as_any()
                .downcast_ref::<UInt32Array>()
                .ok_or_else(|| FormatError::Message("u32 downcast".into()))?;
            Ok(json!(a.value(row)))
        }
        DataType::UInt64 => {
            let a = array
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or_else(|| FormatError::Message("u64 downcast".into()))?;
            Ok(json!(a.value(row)))
        }
        DataType::Float64 => {
            let a = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| FormatError::Message("f64 downcast".into()))?;
            Ok(json!(a.value(row)))
        }
        _ => {
            // Nested / timestamp / list: fall back to Arrow display text.
            let formatter = ArrayFormatter::try_new(array, options)
                .map_err(|e| FormatError::Message(e.to_string()))?;
            Ok(Value::String(formatter.value(row).to_string()))
        }
    }
}

fn csv_escape_json_value(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => csv_escape(s),
        other => csv_escape(&other.to_string()),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Int32Array, StringArray};
    use datafusion::arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    #[test]
    fn batches_to_json_round_trips_primitives() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("state_id", DataType::Int32, false),
            Field::new("good", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("grain"), None])),
            ],
        )
        .unwrap();
        let v = batches_to_json(&[batch]).unwrap();
        assert_eq!(v["row_count"], 2);
        assert_eq!(v["columns"], json!(["state_id", "good"]));
        assert_eq!(v["rows"][0][0], 1);
        assert_eq!(v["rows"][0][1], "grain");
        assert!(v["rows"][1][1].is_null());
    }
}
