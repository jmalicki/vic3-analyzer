use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Float64Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::state_qualifications_schema;
use crate::scope::{state_in_scope, TableScope};

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["state_id"],
    eq_i32: &[],
    eq_str: &["profession"],
    range_str: &[],
};

#[derive(Debug)]
pub struct StateQualificationsProvider {
    binding: Arc<SessionBinding>,
    scope: TableScope,
    schema: SchemaRef,
}

impl StateQualificationsProvider {
    pub fn new(binding: Arc<SessionBinding>, scope: TableScope) -> Self {
        Self {
            binding,
            scope,
            schema: state_qualifications_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut state_id = UInt32Builder::new();
        let mut profession = StringBuilder::new();
        let mut stock = Float64Builder::new();
        let mut jobs = Float64Builder::new();
        let mut shortage = Float64Builder::new();

        for q in &self.binding.prices.state_qualifications {
            if !state_in_scope(self.scope, self.binding.world.as_ref(), Some(q.state_id)) {
                continue;
            }
            if !matches_u32(&preds, "state_id", q.state_id) {
                continue;
            }
            if !matches_str(&preds, "profession", &q.profession_name) {
                continue;
            }
            state_id.append_value(q.state_id);
            profession.append_value(&q.profession_name);
            // `stock` mirrors alert staffing vocabulary (`qualified` stock).
            stock.append_value(q.qualified);
            jobs.append_value(q.jobs);
            shortage.append_value(q.shortage);
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(state_id.finish()),
                Arc::new(profession.finish()),
                Arc::new(stock.finish()),
                Arc::new(jobs.finish()),
                Arc::new(shortage.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for StateQualificationsProvider {
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
