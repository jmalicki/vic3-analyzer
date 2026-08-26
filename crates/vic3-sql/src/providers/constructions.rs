//! Fact table for Vic3 private + government construction queues.
//!
//! Rows come from [`vic3_prices::World::constructions`], not the planning head
//! `queued_building`. Use `queue = 'private'|'government'` and `position` for
//! display order within a country (matching the in-game build queue UI).
//!
//! `building_type_id` is the dense [`vic3_defs::BuildingTypeIdx`] (`UInt16`);
//! `building_type_name` is the localized label when known.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{
    Float64Builder, RecordBatch, StringBuilder, UInt16Builder, UInt32Builder,
};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::constructions_schema;
use crate::scope::{construction_in_scope, TableScope};

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["order_id", "country_id", "state_id"],
    eq_i32: &[],
    eq_str: &["queue"],
    range_str: &[],
};

#[derive(Debug)]
pub struct ConstructionsProvider {
    binding: Arc<SessionBinding>,
    scope: TableScope,
    schema: SchemaRef,
}

impl ConstructionsProvider {
    pub fn new(binding: Arc<SessionBinding>, scope: TableScope) -> Self {
        Self {
            binding,
            scope,
            schema: constructions_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut order_id = UInt32Builder::new();
        let mut queue = StringBuilder::new();
        let mut position = UInt32Builder::new();
        let mut country_id = UInt32Builder::new();
        let mut state_id = UInt32Builder::new();
        let mut building_type_id = UInt16Builder::new();
        let mut building_type_name = StringBuilder::new();
        let mut remaining = Float64Builder::new();

        // Assign position against the full queue first so pushdown filters do
        // not renumber later orders to 0.
        let mut pos_by_key: HashMap<(Option<u32>, &'static str), u32> = HashMap::new();
        let positions: Vec<u32> = self
            .binding
            .world
            .constructions
            .iter()
            .map(|row| {
                let key = (row.country_id, row.queue.as_str());
                let pos = pos_by_key.entry(key).or_insert(0);
                let this_pos = *pos;
                *pos += 1;
                this_pos
            })
            .collect();

        for (row, this_pos) in self.binding.world.constructions.iter().zip(positions) {
            if !construction_in_scope(
                self.scope,
                self.binding.world.as_ref(),
                row.country_id,
                row.state_id,
            ) {
                continue;
            }
            let queue_str = row.queue.as_str();
            if !matches_u32(&preds, "order_id", row.id) {
                continue;
            }
            if !matches_str(&preds, "queue", queue_str) {
                continue;
            }
            // Nullable FKs: equality pushdown excludes NULL rows (SQL NULL ≠ value).
            match row.country_id {
                Some(cid) if !matches_u32(&preds, "country_id", cid) => continue,
                None if preds.iter().any(|p| {
                    matches!(p, crate::filter::Pred::EqU32 { column, .. } if column == "country_id")
                }) =>
                {
                    continue
                }
                _ => {}
            }
            match row.state_id {
                Some(sid) if !matches_u32(&preds, "state_id", sid) => continue,
                None if preds.iter().any(|p| {
                    matches!(p, crate::filter::Pred::EqU32 { column, .. } if column == "state_id")
                }) =>
                {
                    continue
                }
                _ => {}
            }

            let script_id = self
                .binding
                .defs
                .building_by_index(row.building_type_id)
                .unwrap_or("");

            order_id.append_value(row.id);
            queue.append_value(queue_str);
            position.append_value(this_pos);
            match row.country_id {
                Some(id) => country_id.append_value(id),
                None => country_id.append_null(),
            }
            match row.state_id {
                Some(id) => state_id.append_value(id),
                None => state_id.append_null(),
            }
            building_type_id.append_value(row.building_type_id.raw());
            match self.binding.label(script_id) {
                Some(name) => building_type_name.append_value(name),
                None => building_type_name.append_null(),
            }
            match row.remaining {
                Some(value) => remaining.append_value(value),
                None => remaining.append_null(),
            }
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(order_id.finish()),
                Arc::new(queue.finish()),
                Arc::new(position.finish()),
                Arc::new(country_id.finish()),
                Arc::new(state_id.finish()),
                Arc::new(building_type_id.finish()),
                Arc::new(building_type_name.finish()),
                Arc::new(remaining.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for ConstructionsProvider {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        Ok(PUSH.classify(filters))
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        _limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        memory_exec(self.batch(filters)?, projection)
    }
}
