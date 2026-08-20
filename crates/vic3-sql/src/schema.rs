//! Arrow schemas for `docs/sql.md` fact tables.

use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field, Fields, Schema, SchemaRef, TimeUnit};

/// `List<Struct{good, good_name, qty}>` for building / PM IO columns.
pub fn good_io_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", good_io_struct_type(), true)))
}

pub fn good_io_struct_type() -> DataType {
    DataType::Struct(Fields::from(vec![
        Field::new("good", DataType::Utf8, false),
        Field::new("good_name", DataType::Utf8, true),
        Field::new("qty", DataType::Float64, false),
    ]))
}

pub fn text_list_type() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Utf8, true)))
}

pub fn states_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("state_id", DataType::UInt32, false),
        Field::new("region_id", DataType::Utf8, true),
        Field::new("region_name", DataType::Utf8, true),
        Field::new("owner_tag", DataType::Utf8, true),
        Field::new("market_id", DataType::UInt32, true),
        Field::new("infrastructure", DataType::Float64, true),
        Field::new("arable_land", DataType::Float64, true),
    ]))
}

pub fn goods_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("good", DataType::Utf8, false),
        Field::new("good_name", DataType::Utf8, true),
        Field::new("base", DataType::Float64, false),
        Field::new("price", DataType::Float64, false),
        Field::new("buy", DataType::Float64, false),
        Field::new("sell", DataType::Float64, false),
        Field::new("shortage", DataType::Float64, false),
    ]))
}

pub fn goods_by_state_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("state_id", DataType::UInt32, false),
        Field::new("good", DataType::Utf8, false),
        Field::new("price", DataType::Float64, false),
        Field::new("buy", DataType::Float64, false),
        Field::new("sell", DataType::Float64, false),
        Field::new("shortage", DataType::Float64, false),
    ]))
}

pub fn buildings_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("building_id", DataType::UInt32, false),
        Field::new("state_id", DataType::UInt32, true),
        Field::new("type_id", DataType::Utf8, false),
        Field::new("type_name", DataType::Utf8, true),
        Field::new("level", DataType::Float64, false),
        Field::new("staffing", DataType::Float64, false),
        Field::new("employees", DataType::Float64, false),
        Field::new("profit", DataType::Float64, false),
        Field::new("revenue", DataType::Float64, false),
        Field::new("cost", DataType::Float64, false),
        Field::new("production_methods", text_list_type(), false),
        Field::new("short_inputs", text_list_type(), false),
        Field::new("input_goods", good_io_list_type(), false),
        Field::new("output_goods", good_io_list_type(), false),
    ]))
}

pub fn building_types_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("type_id", DataType::Utf8, false),
        Field::new("type_name", DataType::Utf8, true),
        Field::new("group_id", DataType::Utf8, true),
        Field::new("city_type", DataType::Utf8, true),
        Field::new("production_method_groups", text_list_type(), false),
    ]))
}

pub fn production_methods_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("pm", DataType::Utf8, false),
        Field::new("pm_name", DataType::Utf8, true),
        Field::new("inputs", good_io_list_type(), false),
        Field::new("outputs", good_io_list_type(), false),
    ]))
}

pub fn pops_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("state_id", DataType::UInt32, false),
        Field::new("profession", DataType::Utf8, true),
        Field::new("workforce", DataType::Float64, true),
        Field::new("dependents", DataType::Float64, true),
        Field::new("literacy", DataType::Float64, true),
    ]))
}

pub fn state_qualifications_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("state_id", DataType::UInt32, false),
        Field::new("profession", DataType::Utf8, false),
        Field::new("stock", DataType::Float64, false),
        Field::new("jobs", DataType::Float64, false),
        Field::new("shortage", DataType::Float64, false),
    ]))
}

pub fn countries_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("country_id", DataType::UInt32, false),
        Field::new("tag", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
    ]))
}

/// `plan(goal [, …])` columns (`docs/sql.md`).
pub fn plan_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("step", DataType::Int32, false),
        Field::new("day", DataType::UInt32, false),
        Field::new("action", DataType::Utf8, false),
        Field::new("detail", DataType::Utf8, false),
        Field::new("limitations", DataType::Utf8, true),
    ]))
}

/// `gaps(goal)` columns (`docs/sql.md`).
pub fn gaps_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("predicate", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("detail", DataType::Utf8, false),
    ]))
}

pub fn saves_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new(
            "mtime",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        ),
        Field::new("in_game_date", DataType::Utf8, true),
        Field::new("country", DataType::Utf8, true),
        Field::new("location", DataType::Utf8, false),
        Field::new("loaded", DataType::Boolean, false),
    ]))
}
