use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::countries_schema;
use crate::scope::{country_tag_in_scope, TableScope};

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["country_id"],
    eq_i32: &[],
    eq_str: &["country_name"],
    range_str: &[],
};

#[derive(Debug)]
pub struct CountriesProvider {
    binding: Arc<SessionBinding>,
    scope: TableScope,
    schema: SchemaRef,
}

impl CountriesProvider {
    pub fn new(binding: Arc<SessionBinding>, scope: TableScope) -> Self {
        Self {
            binding,
            scope,
            schema: countries_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut country_id = UInt32Builder::new();
        let mut country_name = StringBuilder::new();
        let mut country_label = StringBuilder::new();

        for c in &self.binding.prices.countries {
            if !country_tag_in_scope(self.scope, self.binding.world.as_ref(), &c.country_name) {
                continue;
            }
            if !matches_u32(&preds, "country_id", c.id) {
                continue;
            }
            if !matches_str(&preds, "country_name", &c.country_name) {
                continue;
            }
            country_id.append_value(c.id);
            country_name.append_value(&c.country_name);
            match &c.country_label {
                Some(n) => country_label.append_value(n),
                None => country_label.append_null(),
            }
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(country_id.finish()),
                Arc::new(country_name.finish()),
                Arc::new(country_label.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for CountriesProvider {
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
