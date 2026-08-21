use std::any::Any;
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

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["country_id"],
    eq_i32: &[],
    eq_str: &["tag"],
    range_str: &[],
};

#[derive(Debug)]
pub struct CountriesProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
}

impl CountriesProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self {
            binding,
            schema: countries_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut country_id = UInt32Builder::new();
        let mut tag = StringBuilder::new();
        let mut name = StringBuilder::new();

        for c in &self.binding.prices.countries {
            if !matches_u32(&preds, "country_id", c.id) {
                continue;
            }
            if !matches_str(&preds, "tag", &c.tag) {
                continue;
            }
            country_id.append_value(c.id);
            tag.append_value(&c.tag);
            match &c.name {
                Some(n) => name.append_value(n),
                None => name.append_null(),
            }
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(country_id.finish()),
                Arc::new(tag.finish()),
                Arc::new(name.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for CountriesProvider {
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
