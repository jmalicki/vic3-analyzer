use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{RecordBatch, StringBuilder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::building_types_schema;

use super::arrays::text_list_column;
use super::pushdown::{matches_str, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &[],
    eq_i32: &[],
    eq_str: &["name", "label", "group_id"],
    // defs.building_types is a BTreeMap — Exact range on name.
    range_str: &["name"],
};

#[derive(Debug)]
pub struct BuildingTypesProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
}

impl BuildingTypesProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self {
            binding,
            schema: building_types_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut name = StringBuilder::new();
        let mut label = StringBuilder::new();
        let mut group_id = StringBuilder::new();
        let mut city_type = StringBuilder::new();
        let mut groups: Vec<Vec<String>> = Vec::new();

        for (script_id, bt) in &self.binding.defs.building_types {
            if !matches_str(&preds, "name", script_id) {
                continue;
            }
            let loc = self.binding.label(script_id);
            if !matches_str(&preds, "label", &loc) {
                continue;
            }
            if let Some(g) = &bt.group {
                if !matches_str(&preds, "group_id", g) {
                    continue;
                }
            } else if preds.iter().any(
                |p| matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "group_id"),
            ) {
                continue;
            }

            name.append_value(script_id);
            label.append_value(&loc);
            match &bt.group {
                Some(g) => group_id.append_value(g),
                None => group_id.append_null(),
            }
            match &bt.city_type {
                Some(c) => city_type.append_value(c),
                None => city_type.append_null(),
            }
            groups.push(bt.production_method_groups.clone());
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(name.finish()),
                Arc::new(label.finish()),
                Arc::new(group_id.finish()),
                Arc::new(city_type.finish()),
                text_list_column(&groups),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for BuildingTypesProvider {
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
