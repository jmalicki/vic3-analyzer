use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Float64Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::GoodFlow;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::buildings_schema;

use super::arrays::{good_io_list_column, text_list_column};
use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["building_id", "state_id"],
    eq_str: &["type_id"],
    range_str: &[],
};

#[derive(Debug)]
pub struct BuildingsProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
}

impl BuildingsProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self {
            binding,
            schema: buildings_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut building_id = UInt32Builder::new();
        let mut state_id = UInt32Builder::new();
        let mut type_id = StringBuilder::new();
        let mut type_name = StringBuilder::new();
        let mut level = Float64Builder::new();
        let mut staffing = Float64Builder::new();
        let mut employees = Float64Builder::new();
        let mut profit = Float64Builder::new();
        let mut revenue = Float64Builder::new();
        let mut cost = Float64Builder::new();
        let mut pms: Vec<Vec<String>> = Vec::new();
        let mut shorts: Vec<Vec<String>> = Vec::new();
        let mut inputs: Vec<Vec<GoodFlow>> = Vec::new();
        let mut outputs: Vec<Vec<GoodFlow>> = Vec::new();

        let mut rows = self.binding.prices.buildings.clone();
        rows.sort_by_key(|b| b.id);

        for b in &rows {
            if !matches_u32(&preds, "building_id", b.id) {
                continue;
            }
            if let Some(sid) = b.state_id {
                if !matches_u32(&preds, "state_id", sid) {
                    continue;
                }
            } else if preds.iter().any(
                |p| matches!(p, crate::filter::Pred::EqU32 { column, .. } if column == "state_id"),
            ) {
                continue;
            }
            if !matches_str(&preds, "type_id", &b.type_id) {
                continue;
            }

            building_id.append_value(b.id);
            match b.state_id {
                Some(s) => state_id.append_value(s),
                None => state_id.append_null(),
            }
            type_id.append_value(&b.type_id);
            match self.binding.label(&b.type_id) {
                Some(n) => type_name.append_value(n),
                None => type_name.append_null(),
            }
            level.append_value(b.level);
            staffing.append_value(b.staffing);
            employees.append_value(b.employees.iter().map(|e| e.count).sum::<f64>());
            profit.append_value(b.profit);
            revenue.append_value(b.revenue);
            cost.append_value(b.cost);
            pms.push(b.production_method_ids.clone());
            shorts.push(b.short_inputs.clone());
            inputs.push(b.inputs.clone());
            outputs.push(b.outputs.clone());
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(building_id.finish()),
                Arc::new(state_id.finish()),
                Arc::new(type_id.finish()),
                Arc::new(type_name.finish()),
                Arc::new(level.finish()),
                Arc::new(staffing.finish()),
                Arc::new(employees.finish()),
                Arc::new(profit.finish()),
                Arc::new(revenue.finish()),
                Arc::new(cost.finish()),
                text_list_column(&pms),
                text_list_column(&shorts),
                good_io_list_column(&self.binding, &inputs),
                good_io_list_column(&self.binding, &outputs),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for BuildingsProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

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
